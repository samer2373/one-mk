use anyhow::{Context, Result, bail, ensure};
use one_mk_ctl::status::Status;
use one_mk_net::{ControlWriter, Fingerprint, Node};
use one_mk_proto::{Control, StreamKind};
use one_mk_wallpaper::{Command, Media, Plan, Playback, PlayerConfig, Wire, cache_path, clock_offset, now_ms};
#[cfg(test)]
use one_mk_wallpaper::prune_cache;
use ring::digest::{Context as Digest, SHA256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub struct Peer { pub id: String, pub name: String, pub fingerprint: Fingerprint, pub address: std::net::SocketAddr, pub control: ControlWriter }

pub struct Layout { pub computer: String, pub canvas: one_mk_wallpaper::Canvas, pub paired: BTreeSet<String> }
impl Layout {
    pub fn from_status(status: &Status) -> Self {
        Self { computer: status.id.clone(), canvas: one_mk_ctl::wallpaper::canvas(status), paired: status.peers.iter().map(|peer| peer.id.clone()).collect() }
    }
}

fn same_playback_plan(first: &Plan, second: &Plan) -> bool {
    let mut first = first.clone();
    let mut second = second.clone();
    for monitor in first.canvas.monitors.iter_mut().chain(&mut second.canvas.monitors) { monitor.online = true; }
    first == second
}

#[derive(Default)]
struct State {
    status: one_mk_wallpaper::Status,
    player: Option<Child>,
    retiring: Vec<Child>,
    report_to: Option<ControlWriter>,
    clock_wake: Arc<tokio::sync::Notify>,
    generation: u64,
    launch: u64,
    computer: String,
    locked: bool,
    asleep: bool,
    offset: i64,
    best_rtt: Option<i64>,
    coordinator: Option<Fingerprint>,
    previous: Option<String>,
    progress: Option<(String, Arc<crate::transfer::Progress>)>,
    clocks: BTreeMap<Fingerprint, i64>,
    worker: Option<tokio::task::JoinHandle<()>>,
    directory: Option<PathBuf>,
    connections: BTreeSet<Fingerprint>,
    deferred: Option<(PathBuf, Layout, Vec<Peer>, Node)>,
    inbox: Option<Arc<crate::transfer::Inbox>>,
    authority_off: bool,
    pending: Option<(Fingerprint, Plan)>,
    awaiting: BTreeSet<(Fingerprint, String)>,
    applied_picture: Option<Plan>,
    /// A video this computer can't play yet is shown as a still of its first
    /// frame meanwhile (GNOME before its extension is set up).
    still_fallback: bool,
    picture_mode: one_mk_wallpaper::picture::Mode,
    extension_check: Option<std::time::Instant>,
    extension_checking: bool,
    extension_changing: bool,
}

impl State {
    fn replace_player(&mut self, exclusive: bool) {
        if let Some(mut child) = self.player.take() {
            if !exclusive && self.status.computers.get(&self.computer).is_some_and(Playback::is_ready) { self.retiring.push(child); }
            else { let _ = child.kill(); let _ = child.wait(); }
        }
        if exclusive { self.retire(); }
        self.launch += 1;
    }
    fn retire(&mut self) {
        for mut child in self.retiring.drain(..) {
            if let Some(mut input) = child.stdin.take() { let _ = writeln!(input, "quit"); }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    fn stop(&mut self) {
        self.launch += 1;
        if let Some(child) = self.player.take() { self.retiring.push(child); }
        self.retire();
        self.clock_wake.notify_one();
    }
}
impl Drop for State { fn drop(&mut self) { if let Some(worker) = self.worker.take() { worker.abort(); } self.stop(); } }

#[derive(Clone, Default)]
pub struct Runtime { state: Arc<Mutex<State>>, picture_work: Arc<Mutex<()>> }

#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    status: one_mk_wallpaper::Status,
    coordinator: Option<String>,
    previous: Option<String>,
    #[serde(default)]
    authority_off: bool,
}

/// Why nothing plays while the screen is locked or the computer sleeps.
const HELD_MESSAGE: &str = "Paused while this computer is locked or asleep; it resumes when you unlock it";

fn helper() -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    one_mk_ctl::install::sibling(&executable, "one-mk-wallpaper").context("The video wallpaper player is missing from this oneMK installation. Reinstall oneMK to add it.")
}

fn restore_picture(root: &Path) -> Result<()> {
    if !root.join("previous.json").exists() { return Ok(()); }
    one_mk_wallpaper::picture::restore(&helper()?, root)
}

pub fn cache(dir: &Path) -> PathBuf { dir.join("wallpaper-video") }

/// What was chosen can be used: a file that exists, not a folder. Said with
/// the path, so a typo or a folder dropped by mistake reads as what it is.
fn check_source(path: &Path) -> Result<()> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!("{} does not exist. Check the name and folder, then choose it again.", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => bail!("oneMK is not allowed to read {}. Choose a file you can open.", path.display()),
        Err(error) => bail!("Cannot read {}: {error}", path.display()),
    };
    ensure!(!metadata.is_dir(), "{} is a folder. Choose a video or picture file instead.", path.display());
    ensure!(metadata.is_file(), "{} is not a regular file. Choose a video or picture file.", path.display());
    Ok(())
}

/// The error for a video that can't play here, saying when a still of its
/// first frame is shown meanwhile (GNOME before its extension is set up).
fn with_still_note(error: anyhow::Error, still: bool) -> anyhow::Error {
    if still { anyhow::anyhow!("{error} {}", one_mk_wallpaper::GNOME_STILL_SHOWN) } else { error }
}

pub(crate) fn digest(path: &Path) -> Result<String> {
    let mut file = crate::transfer::open_regular(path, std::fs::symlink_metadata(path)?.len())?;
    let before = one_mk_wallpaper::file_identity(&file)?;
    type Verified = BTreeMap<PathBuf, (Vec<u64>, String)>;
    static VERIFIED: std::sync::OnceLock<Mutex<Verified>> = std::sync::OnceLock::new();
    let verified = VERIFIED.get_or_init(Mutex::default);
    if let Some((identity, hash)) = verified.lock().unwrap().get(path) && identity == &before { return Ok(hash.clone()); }
    ensure!(file.metadata()?.is_file(), "Choose a regular video file");
    ensure!(file.metadata()?.len() <= one_mk_wallpaper::MAX_MEDIA_BYTES, "Choose a video smaller than 4 GB");
    let mut hash = Digest::new(&SHA256);
    let mut buffer = vec![0u8; 256 * 1024];
    loop { let count = file.read(&mut buffer)?; if count == 0 { break; } hash.update(&buffer[..count]); }
    ensure!(before == one_mk_wallpaper::file_identity(&file)?, "The wallpaper changed while it was being verified");
    let hash: String = hash.finish().as_ref().iter().map(|byte| format!("{byte:02x}")).collect();
    let mut verified = verified.lock().unwrap();
    if verified.len() >= 256 { verified.clear(); }
    verified.insert(path.to_owned(), (before, hash.clone()));
    Ok(hash)
}

fn import(dir: &Path, path: &Path) -> Result<(String, PathBuf)> {
    let root = cache(dir);
    one_mk_ctl::private::create_dir_all(&root)?;
    let hash = digest(path)?;
    let destination = cache_path(&root, &hash)?;
    if digest(&destination).ok().as_deref() != Some(hash.as_str()) {
        let temporary = root.join(format!("{}.part", crate::link::random_u64()));
        let result = (|| -> Result<()> {
            let mut source = crate::transfer::open_regular(path, std::fs::metadata(path)?.len())?;
            let mut target = one_mk_ctl::private::create_new(&temporary)?;
            std::io::copy(&mut source, &mut target)?;
            target.sync_all()?;
            ensure!(digest(&temporary)? == hash, "The video changed while it was being copied. Choose it again.");
            std::fs::rename(&temporary, &destination)?;
            Ok(())
        })();
        let _ = std::fs::remove_file(temporary);
        result?;
    }
    Ok((hash, destination))
}

/// Removes a video the daemon copied in (and its poster) when it turned out
/// to be unusable, unless it is one of the `keep` hashes still in use.
fn discard_unused(dir: &Path, hash: &str, keep: &BTreeSet<String>) {
    if keep.contains(hash) { return; }
    if let Ok(path) = cache_path(&cache(dir), hash) { let _ = std::fs::remove_file(path); }
    let _ = std::fs::remove_file(cache(dir).join(format!("{hash}.png")));
}

async fn inspect(dir: &Path, hash: &str) -> Result<Media> {
    inspect_file(&cache_path(&cache(dir), hash)?, Some(&cache(dir).join(format!("{hash}.png")))).await
}

async fn inspect_file(path: &Path, poster: Option<&Path>) -> Result<Media> {
    let mut probe = tokio::process::Command::new(helper()?);
    probe.arg("--probe").arg(path).kill_on_drop(true);
    if let Some(poster) = poster { probe.arg(poster); }
    let output = tokio::time::timeout(Duration::from_secs(40), probe.output()).await.context("The system took too long to read this video")??;
    ensure!(output.status.success(), "{}", probe_failure(&String::from_utf8_lossy(&output.stderr)));
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// What the helper said when it could not read a file, as a plain sentence:
/// it ends with Rust's `Error: ` (and, with causes, "Caused by:" lines), which
/// is for developers, not for the person who chose a text file as a video.
fn probe_failure(stderr: &str) -> String {
    let text = stderr.trim();
    let text = text.strip_prefix("Error:").map_or(text, str::trim_start);
    let first = text.split("\n\nCaused by:").next().unwrap_or(text).trim();
    if first.is_empty() { "The system could not read this file as a video or picture. Choose another file.".into() } else { first.to_owned() }
}

/// Saves what is chosen, so it comes back after a restart: written beside,
/// flushed, then swapped in, so a crash leaves the old or the new one whole.
fn save_selection(dir: &Path, saved: &Saved) -> Result<()> {
    use std::io::Write;
    one_mk_ctl::private::create_dir_all(&cache(dir))?;
    let temporary = cache(dir).join("selection.tmp");
    let mut file = one_mk_ctl::private::write_open(&temporary)?;
    file.write_all(&serde_json::to_vec(saved)?)?;
    file.sync_all()?;
    std::fs::rename(temporary, cache(dir).join("selection.json"))?;
    Ok(())
}

struct Prepared(PathBuf);
impl Drop for Prepared {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        // macOS's exporter writes `<name>.sb-…` beside it first; a stopped
        // or failed export leaves that behind.
        let (Some(folder), Some(name)) = (self.0.parent(), self.0.file_name().and_then(|name| name.to_str())) else { return };
        let Ok(entries) = std::fs::read_dir(folder) else { return };
        for entry in entries.flatten() {
            if entry.file_name().to_str().is_some_and(|other| other.starts_with(&format!("{name}.sb-"))) { let _ = std::fs::remove_file(entry.path()); }
        }
    }
}

fn picture_asset(dir: &Path, plan: &Plan, computer: &str) -> Result<String> {
    let key = ring::digest::digest(&SHA256, &serde_json::to_vec(&(3, &plan.hash, &plan.canvas, plan.fit, plan.spread, computer))?);
    let key: String = key.as_ref().iter().map(|byte| format!("{byte:02x}")).collect();
    let index = cache(dir).join(format!("{}-{key}.slice", plan.hash));
    if let Ok(hash) = std::fs::read_to_string(&index) && let Ok(path) = cache_path(&cache(dir), &hash) && digest(&path).ok().as_ref() == Some(&hash) { return Ok(hash); }
    let temporary = cache(dir).join(format!("picture-{}.part", crate::link::random_u64()));
    let result = (|| -> Result<String> {
        one_mk_wallpaper::picture::prepare(&helper()?, &cache_path(&cache(dir), &plan.hash)?, plan, computer, &temporary)?;
        Ok(import(dir, &temporary)?.0)
    })();
    let _ = std::fs::remove_file(temporary);
    if let Ok(hash) = &result { one_mk_ctl::private::write(&index, hash)?; }
    result
}

/// Whether a video this computer can't play can show a still of its first
/// frame instead, through the same native picture path a chosen picture uses
/// (restorable, no player). False when video plays here, this computer isn't
/// selected, or pictures can't be set either (GNOME before its extension).
fn can_show_still(plan: &Plan, computer: &str) -> bool {
    !plan.picture && plan.computers.iter().any(|id| id == computer)
        && one_mk_wallpaper::availability().is_err() && one_mk_wallpaper::picture::available()
}

/// Cuts and sets the video's first frame for this computer's screens. Returns
/// whether it is a native wallpaper (an overlay would need a running helper,
/// so that case is refused rather than started).
fn apply_still(dir: &Path, plan: &Plan, computer: &str) -> Result<bool> {
    let video = cache_path(&cache(dir), &plan.hash)?;
    let key = format!("{}-{}", plan.hash, crate::link::random_u64());
    let frame = cache(dir).join(format!("still-{key}.png"));
    let bundle = cache(dir).join(format!("still-{key}.part"));
    let result = (|| -> Result<bool> {
        // Large enough for this computer's own span of the canvas at its scale.
        let width = plan.canvas.monitors.iter().filter(|monitor| monitor.computer == computer)
            .map(|monitor| (monitor.local.unwrap_or(monitor.rect).width * monitor.scale).ceil() as u32).sum::<u32>().clamp(1280, 3840);
        let media = one_mk_wallpaper::picture::still(&helper()?, &video, &frame, width)?;
        let still = Plan { picture: true, asset_hash: None, media, ..plan.clone() };
        one_mk_wallpaper::picture::prepare(&helper()?, &frame, &still, computer, &bundle)?;
        Ok(one_mk_wallpaper::picture::apply(&helper()?, &still, computer, &bundle, &cache(dir).join("pictures"))? == one_mk_wallpaper::picture::Mode::Native)
    })();
    let _ = std::fs::remove_file(frame);
    let _ = std::fs::remove_file(bundle);
    result
}

fn prune_selection(dir: &Path, plan: &Plan, previous: Option<&str>) -> Result<()> {
    let root = cache(dir);
    let mut keep = BTreeSet::from([plan.hash.clone()]);
    keep.extend(previous.map(str::to_owned));
    keep.extend(plan.asset_hash.clone());
    for entry in std::fs::read_dir(&root)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "slice") {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with(&plan.hash) || previous.is_some_and(|hash| name.starts_with(hash)) {
                if let Ok(hash) = std::fs::read_to_string(&path) { keep.insert(hash); }
            } else { std::fs::remove_file(path)?; }
        }
    }
    one_mk_wallpaper::prune_cache_except(&root, &keep)?;
    Ok(())
}

impl Runtime {
    async fn prepare(&self, dir: &Path, path: &Path, canvas: &one_mk_wallpaper::Canvas, generation: u64) -> Result<Option<Prepared>> {
        use tokio::io::AsyncBufReadExt;
        let media = inspect_file(path, None).await?;
        let bytes = std::fs::metadata(path)?.len();
        ensure!(bytes <= one_mk_wallpaper::MAX_MEDIA_BYTES, "Choose a video smaller than 4 GB");
        let Some(target) = one_mk_wallpaper::ImportTarget::for_canvas(canvas, &media, bytes) else { return Ok(None) };
        one_mk_ctl::private::create_dir_all(&cache(dir))?;
        let prepared = Prepared(cache(dir).join(format!("prepared-{}.mp4", crate::link::random_u64())));
        let log = one_mk_ctl::private::write_open(&cache(dir).join("prepare.log"))?;
        let mut child = tokio::process::Command::new(helper()?).arg("--prepare").arg(path).arg(&prepared.0).arg(serde_json::to_string(&target)?).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(log).kill_on_drop(true).spawn()?;
        let mut lines = tokio::io::BufReader::new(child.stdout.take().context("The video preparation did not start")?).lines();
        tokio::time::timeout(Duration::from_secs(600), async {
            while let Some(line) = lines.next_line().await? {
                if let Some(percent) = line.strip_prefix("progress ").and_then(|text| text.parse::<u32>().ok()) {
                    let mut state = self.state.lock().unwrap();
                    ensure!(state.generation == generation, "Wallpaper selection changed");
                    state.status.message = format!("Making a smaller copy of the video (experimental)… {}%", percent.min(100));
                }
            }
            ensure!(child.wait().await?.success(), "The system could not prepare a smaller copy");
            anyhow::Ok(())
        }).await.context("The system took too long to prepare this video")??;
        let result = inspect_file(&prepared.0, None).await?;
        ensure!(result.width <= media.width && result.height <= media.height && result.frame_rate <= 60, "The prepared video was not the right size or speed");
        Ok(Some(prepared))
    }

    pub fn inbox(&self, dir: &Path) -> Arc<crate::transfer::Inbox> {
        self.state.lock().unwrap().inbox.get_or_insert_with(|| crate::transfer::Inbox::with_notifier(cache(dir), Arc::new(crate::transfer::Quiet))).clone()
    }

    pub fn restore(&self, dir: &Path, computer: String) {
        let work_root = cache(dir);
        let prune = move || {
            let saved = std::fs::read(work_root.join("selection.json")).ok().and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok());
            let mut keep = BTreeSet::new();
            if let Some(saved) = saved && let Some(plan) = saved.status.plan && plan.valid() {
                keep.insert(plan.hash); keep.extend(plan.asset_hash); keep.extend(saved.previous);
            }
            let _ = one_mk_wallpaper::prune_cache_except(&work_root, &keep);
            let _ = std::fs::remove_dir_all(work_root.join(crate::transfer::PARTIAL_DIR));
        };
        prune();
        let saved = std::fs::read(cache(dir).join("selection.json")).ok().and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
            .filter(|saved| saved.status.plan.as_ref().is_none_or(Plan::valid));
        // Nothing chosen, yet the person's own picture is still saved: oneMK
        // stopped (a crash, a power cut, an uninstall without a daemon) before
        // it was put back. It is put back now.
        if saved.as_ref().is_none_or(|saved| saved.status.plan.is_none()) && cache(dir).join("pictures/previous.json").exists() {
            let (runtime, root) = (self.clone(), cache(dir).join("pictures"));
            std::thread::spawn(move || {
                let _work = runtime.picture_work.lock().unwrap();
                if runtime.state.lock().unwrap().status.plan.is_some() { return; }
                if let Err(error) = restore_picture(&root) { tracing::warn!("Could not put your own wallpaper back: {error:#}"); }
            });
        }
        let Some(saved) = saved else { return };
        let mut state = self.state.lock().unwrap();
        state.computer = computer;
        state.coordinator = saved.coordinator.and_then(|fingerprint| fingerprint.parse().ok());
        state.previous = saved.previous;
        state.authority_off = saved.authority_off;
        state.status = saved.status;
        state.status.computers.clear();
        let picture = state.status.plan.as_ref().filter(|plan| plan.picture).cloned();
        if picture.is_none() && state.coordinator.is_none() && let Err(error) = Self::start(dir, &mut state) {
            let still = state.status.plan.as_ref().is_some_and(|plan| can_show_still(plan, &state.computer)) && cache(dir).join("pictures/previous.json").exists();
            state.status.message = with_still_note(error, still).to_string();
        }
        let generation = state.generation;
        let coordinator = state.coordinator;
        let computer = state.computer.clone();
        drop(state);
        if let Some(plan) = picture {
            let runtime = self.clone(); let dir = dir.to_owned();
            tokio::spawn(async move { if let Err(error) = runtime.commit(&dir, plan, computer, coordinator, generation).await { runtime.state.lock().unwrap().status.message = error.to_string(); } });
        }
        self.listen();
    }

    pub fn needs_reconcile(&self) -> bool { let state = self.state.lock().unwrap(); state.coordinator.is_none() && (state.status.plan.is_some() || state.authority_off) }

    pub fn reconcile(&self, dir: PathBuf, layout: Layout, peers: Vec<Peer>, node: Node) {
        let mut state = self.state.lock().unwrap();
        if state.coordinator.is_some() { return; }
        if state.authority_off {
            let connections = peers.iter().map(|peer| peer.fingerprint).collect();
            if state.connections != connections {
                for peer in &peers { send(&peer.control, &Wire::Off); }
                state.connections = connections;
            }
            return;
        }
        if state.worker.as_ref().is_some_and(|worker| !worker.is_finished()) {
            state.deferred = Some((dir, layout, peers, node));
            return;
        }
        let Some(plan) = state.status.plan.as_ref() else { return };
        let mut updated = plan.clone();
        updated.canvas.monitors.retain(|monitor| monitor.computer == layout.computer || layout.paired.contains(&monitor.computer));
        updated.computers.retain(|computer| computer == &layout.computer || layout.paired.contains(computer));
        updated.canvas.update(layout.canvas.monitors);
        let connections = peers.iter().map(|peer| peer.fingerprint).collect();
        let changed = !same_playback_plan(&updated, plan);
        let peers: Vec<_> = peers.into_iter().filter(|peer| changed || !state.connections.contains(&peer.fingerprint)).collect();
        state.connections = connections;
        if !changed && peers.is_empty() { return; }
        if changed { state.generation += 1; }
        let generation = state.generation;
        drop(state);
        let runtime = self.clone();
        let mut state = self.state.lock().unwrap();
        let task = tokio::spawn(async move {
            let result = if changed { runtime.commit(&dir, updated.clone(), layout.computer, None, generation).await } else { Ok(()) };
            match result {
                Err(error) => tracing::warn!("Wallpaper layout: {error}"),
                Ok(()) => for peer in peers {
                    if let Err(error) = runtime.distribute(&dir, &updated, &peer, &node, generation).await { tracing::warn!("Wallpaper copy: {error}"); }
                }
            }
            runtime.finish_worker(generation);
        });
        state.worker = Some(task);
    }

    fn finish_worker(&self, generation: u64) {
        let deferred = {
            let mut state = self.state.lock().unwrap();
            if state.generation != generation { return; }
            state.worker = None;
            state.progress = None;
            state.deferred.take()
        };
        if let Some((dir, layout, peers, node)) = deferred { self.reconcile(dir, layout, peers, node); }
    }

    pub fn status(&self) -> one_mk_wallpaper::Status {
        self.refresh_extension();
        let mut state = self.state.lock().unwrap();
        if let Some(child) = &mut state.player && let Ok(Some(code)) = child.try_wait() {
            state.player = None;
            state.status.message = format!("The wallpaper player stopped ({code}). Choose Play to try again.");
            if let Some(directory) = &state.directory && let Ok(log) = std::fs::read_to_string(cache(directory).join("player.log")) {
                let detail: String = log.lines().next().unwrap_or_default().chars().take(300).collect();
                if !detail.is_empty() { state.status.message = detail; }
            }
        }
        let mut status = state.status.clone();
        // Counted from what each computer said, not from the plan alone.
        if let Some(plan) = status.plan.as_ref() && status.message == plan.playing_message() { status.message = plan.playing_summary(&status.computers); }
        if let Some(Playback::Paused(message) | Playback::Error(message)) = status.computers.get(&state.computer) { status.message = message.clone(); }
        if status.computers.get(&state.computer) == Some(&Playback::Picture) { status.message = Playback::Picture.message().into(); }
        else if status.plan.as_ref().is_some_and(|plan| plan.picture) && status.computers.values().any(|playback| playback == &Playback::Picture) {
            status.message += ". Some computers show this picture with oneMK; their own wallpaper returns when you turn it off";
        }
        if let Some((name, progress)) = &state.progress {
            use std::sync::atomic::Ordering::Relaxed;
            let total = progress.total.load(Relaxed);
            let done = progress.done.load(Relaxed);
            status.message = format!("Copying to {name}… {}%", done.saturating_mul(100).checked_div(total).unwrap_or(0));
        }
        if let Err(reason) = one_mk_wallpaper::availability() && !status.plan.as_ref().is_some_and(|plan| plan.picture) && !one_mk_wallpaper::picture::available() {
            status.available = false;
            status.message = reason.into();
        } else { status.available = true; }
        let sharing = status.plan.as_ref().is_some_and(|plan| plan.computers.iter().any(|computer| computer != &state.computer));
        status.experimental = one_mk_wallpaper::experimental_note(sharing);
        status
    }

    fn refresh_extension(&self) {
        if !cfg!(target_os = "linux") || !one_mk_wallpaper::gnome_wayland() { return; }
        let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
        let mut state = self.state.lock().unwrap();
        if state.extension_checking || state.extension_changing || state.extension_check.is_some_and(|last| last.elapsed() < Duration::from_secs(2)) { return; }
        state.extension_checking = true;
        let shared = Arc::downgrade(&self.state);
        handle.spawn(async move {
            let setup = tokio::time::timeout(Duration::from_secs(4), crate::wallpaper::extension::setup()).await;
            let setup = match setup {
                Ok(Ok(setup)) => setup,
                _ => one_mk_wallpaper::GnomeSetup { stage: one_mk_wallpaper::GnomeStage::Unavailable,
                    message: "Could not check the GNOME extension. Open oneMK in your GNOME Wayland session and try again.".into(), can_install: false, can_enable: false },
            };
            if let Some(shared) = shared.upgrade() {
                let mut state = shared.lock().unwrap();
                state.extension_checking = false;
                state.extension_check = Some(std::time::Instant::now());
                if !state.extension_changing { state.status.gnome = Some(setup); }
            }
        });
    }

    pub fn shutdown(&self) {
        let mut state = self.state.lock().unwrap();
        state.generation += 1;
        if let Some(worker) = state.worker.take() { worker.abort(); }
        state.deferred = None;
        state.stop();
    }

    pub fn hold(&self, dir: &Path, locked: Option<bool>, asleep: Option<bool>) {
        let mut state = self.state.lock().unwrap();
        if let Some(locked) = locked { state.locked = locked; }
        if let Some(asleep) = asleep { state.asleep = asleep; }
        if state.player.is_none() && state.status.plan.as_ref().is_none_or(|plan| plan.picture && state.picture_mode == one_mk_wallpaper::picture::Mode::Native) { return; }
        if state.locked || state.asleep {
            state.stop();
            let computer = state.computer.clone();
            let playback = Playback::Paused(HELD_MESSAGE.into());
            state.status.computers.insert(computer, playback.clone());
            if let Some(control) = &state.report_to && let Some(plan) = &state.status.plan { send(control, &Wire::Playback { hash: plan.hash.clone(), playback }); }
        }
        else if let Err(error) = Self::start(dir, &mut state) { state.status.message = error.to_string(); }
        drop(state);
        self.listen();
    }

    fn start(dir: &Path, state: &mut State) -> Result<()> {
        state.directory = Some(dir.to_owned());
        let Some(plan) = state.status.plan.as_ref() else { return Ok(()) };
        if !plan.computers.contains(&state.computer) {
            state.stop();
            state.status.computers.remove(&state.computer);
            state.status.message = "Wallpaper is off on this computer".into();
            return Ok(());
        }
        if plan.picture && state.picture_mode == one_mk_wallpaper::picture::Mode::Native {
            state.status.computers.insert(state.computer.clone(), Playback::Ready);
            state.status.message = "Picture wallpaper is set".into();
            state.retire();
            return Ok(());
        }
        // Paused by the person, a playing wallpaper holds its frame (the
        // player pauses itself); nothing starts one just to show a still.
        let held = !plan.picture && plan.paused && state.player.is_some();
        if state.locked || state.asleep || (!plan.picture && plan.paused && !held) {
            state.stop();
            // Say why: a wallpaper set while the screen is locked is paused
            // by the lock, not by anyone.
            let why = if state.locked || state.asleep { HELD_MESSAGE } else { "Paused" };
            state.status.computers.insert(state.computer.clone(), Playback::Paused(why.into()));
            state.status.message = why.into();
            return Ok(());
        }
        if let Some(child) = &mut state.player {
            if let Some(input) = child.stdin.as_mut() { let _ = writeln!(input, "{}", if held { "pause" } else { "play" }); }
            if held {
                state.status.computers.insert(state.computer.clone(), Playback::Paused("Paused".into()));
                state.status.message = "Paused".into();
            } else if let Some(plan) = state.status.plan.as_ref() { state.status.message = plan.playing_message(); }
            return Ok(());
        }
        if !plan.picture { one_mk_wallpaper::availability().map_err(anyhow::Error::msg)?; }
        let config = PlayerConfig { plan: plan.clone(), computer: state.computer.clone(), path: cache_path(&cache(dir), if plan.picture { plan.asset_hash.as_ref().unwrap_or(&plan.hash) } else { &plan.hash })?, clock_offset_ms: state.offset };
        let config_path = cache(dir).join("player.json");
        let _ = std::fs::remove_file(cache(dir).join("playback.txt"));
        one_mk_ctl::private::write(&config_path, serde_json::to_vec(&config)?)?;
        let log = one_mk_ctl::private::write_open(&cache(dir).join("player.log"))?;
        state.launch += 1;
        state.player = Some(std::process::Command::new(helper()?).arg(config_path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(log).spawn()?);
        state.status.computers.insert(state.computer.clone(), Playback::Preparing);
        state.clock_wake.notify_one();
        state.status.message = plan.playing_message();
        Ok(())
    }

    async fn commit(&self, dir: &Path, plan: Plan, computer: String, coordinator: Option<Fingerprint>, generation: u64) -> Result<()> {
        let runtime = self.clone();
        let dir = dir.to_owned();
        tokio::task::spawn_blocking(move || runtime.commit_sync(&dir, plan, computer, coordinator, generation)).await?
    }

    fn commit_sync(&self, dir: &Path, mut plan: Plan, computer: String, coordinator: Option<Fingerprint>, generation: u64) -> Result<()> {
        let _work = self.picture_work.lock().unwrap();
        ensure!(plan.valid(), "The wallpaper layout or video is invalid");
        ensure!(serde_json::to_vec(&Wire::Plan { plan: plan.clone() })?.len() <= 12000, "This wallpaper layout has too many monitors to share");
        if plan.picture && coordinator.is_none() && plan.computers.contains(&computer) { plan.asset_hash = Some(picture_asset(dir, &plan, &computer)?); }
        {
            let state = self.state.lock().unwrap();
            ensure!(state.generation == generation, "Wallpaper selection changed");
            if state.coordinator == coordinator && state.status.plan.as_ref() == Some(&plan) && (state.picture_mode == one_mk_wallpaper::picture::Mode::Native && state.applied_picture.as_ref() == Some(&plan) || state.player.is_some() || state.still_fallback) { return Ok(()); }
        }
        let previous_mode = {
            let state = self.state.lock().unwrap();
            (state.coordinator == coordinator && state.applied_picture.as_ref().is_some_and(|old|
                Plan { paused: plan.paused, pause_on_battery: plan.pause_on_battery, ..old.clone() } == plan)).then_some(state.picture_mode)
        };
        let mut still_shown = false;
        let mode = if plan.picture && plan.computers.contains(&computer) {
            if let Some(mode) = previous_mode { mode } else {
                one_mk_wallpaper::picture::apply(&helper()?, &plan, &computer, &cache_path(&cache(dir), plan.asset_hash.as_ref().unwrap_or(&plan.hash))?, &cache(dir).join("pictures"))?
            }
        } else if can_show_still(&plan, &computer) {
            // Setting a still can fail (no gsettings, odd poster): the video's
            // own error still reaches the owner, just without a still.
            match apply_still(dir, &plan, &computer) {
                Ok(shown) => { still_shown = shown; }
                Err(error) => { eprintln!("wallpaper: could not show a still of the video: {error:#}"); restore_picture(&cache(dir).join("pictures"))?; }
            }
            one_mk_wallpaper::picture::Mode::Native
        } else { restore_picture(&cache(dir).join("pictures"))?; one_mk_wallpaper::picture::Mode::Native };
        let mut state = self.state.lock().unwrap();
        ensure!(state.generation == generation, "Wallpaper selection changed");
        // Only paused or played: the running player is told (it holds or
        // resumes its frame); anything else starts a player for the new plan.
        let same_but_paused = state.coordinator == coordinator && state.player.is_some()
            && state.status.plan.as_ref().is_some_and(|old| Plan { paused: plan.paused, ..old.clone() } == plan);
        if !same_but_paused {
            state.replace_player(one_mk_wallpaper::gnome_wayland());
        }
        state.previous = state.status.plan.as_ref().map(|old| old.hash.clone()).filter(|hash| hash != &plan.hash).or(state.previous.take());
        state.computer = computer;
        if state.coordinator != coordinator && coordinator.is_none() { state.offset = 0; state.best_rtt = None; state.clocks.clear(); }
        state.coordinator = coordinator;
        if coordinator.is_none() { state.report_to = None; }
        state.authority_off = false;
        let hash = plan.hash.clone();
        state.applied_picture = plan.picture.then(|| plan.clone());
        state.picture_mode = mode;
        state.still_fallback = still_shown;
        state.status.plan = Some(plan);
        let selected = state.status.plan.as_ref().unwrap().computers.clone();
        state.status.computers.retain(|computer, _| selected.contains(computer));
        state.progress = None;
        state.status.poster = cache(dir).join(format!("{hash}.png")).exists().then(|| cache(dir).join(format!("{hash}.png")));
        let saved = Saved { status: state.status.clone(), coordinator: state.coordinator.map(|fingerprint| fingerprint.to_string()), previous: state.previous.clone(), authority_off: false };
        save_selection(dir, &saved)?;
        prune_selection(dir, state.status.plan.as_ref().unwrap(), state.previous.as_deref())?;
        if let Err(error) = Self::start(dir, &mut state) {
            let computer = state.computer.clone();
            let error = with_still_note(error, still_shown);
            state.status.computers.insert(computer, Playback::Error(error.to_string()));
            return Err(error);
        }
        drop(state);
        self.listen();
        Ok(())
    }

    fn listen(&self) {
        let mut state = self.state.lock().unwrap();
        let Some(output) = state.player.as_mut().and_then(|child| child.stdout.take()) else { return };
        let launch = state.launch;
        let shared = Arc::downgrade(&self.state);
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                let Ok(playback) = serde_json::from_str::<Playback>(&line) else { continue };
                let Some(shared) = shared.upgrade() else { return };
                let mut state = shared.lock().unwrap();
                if state.launch != launch { return; }
                if playback.is_ready() { state.retire(); }
                let computer = state.computer.clone();
                state.status.computers.insert(computer, playback.clone());
                state.clock_wake.notify_one();
                if let Some(control) = &state.report_to && let Some(plan) = &state.status.plan { send(control, &Wire::Playback { hash: plan.hash.clone(), playback }); }
            }
            let Some(shared) = shared.upgrade() else { return };
            let mut state = shared.lock().unwrap();
            if state.launch != launch { return; }
            if let Some(mut child) = state.player.take() { let _ = child.wait(); }
            state.clock_wake.notify_one();
            if matches!(state.status.computers.get(&state.computer), Some(Playback::Error(_))) { return; }
            let playback = Playback::Error("The wallpaper player stopped. Choose Play to try again.".into());
            let computer = state.computer.clone();
            state.status.computers.insert(computer, playback.clone());
            if let Some(control) = &state.report_to && let Some(plan) = &state.status.plan { send(control, &Wire::Playback { hash: plan.hash.clone(), playback }); }
        });
    }

    pub fn request(&self, dir: PathBuf, status: Status, command: Command, peers: Vec<Peer>, node: Node, reply: tokio::sync::oneshot::Sender<String>) {
        // Setup is local consent, not a wallpaper-plan mutation. It must not
        // cancel a transfer, increment its generation or tell any peer to play.
        if let Command::GnomeExtension { operation } = command {
            let mut state = self.state.lock().unwrap();
            if state.extension_changing { let _ = reply.send("error GNOME extension setup is already running\n".into()); return; }
            state.extension_changing = true;
            let shared = Arc::downgrade(&self.state);
            tokio::spawn(async move {
                let result = crate::wallpaper::extension::consented_action(operation).await;
                if let Some(shared) = shared.upgrade() {
                    let mut state = shared.lock().unwrap();
                    state.extension_changing = false;
                    state.extension_check = None;
                    if let Err(error) = &result {
                        state.status.gnome = Some(one_mk_wallpaper::GnomeSetup { stage: one_mk_wallpaper::GnomeStage::Failed,
                            message: format!("GNOME extension setup: {error:#}"), can_install: false, can_enable: false });
                    }
                }
                let _ = reply.send(result.map_or_else(|error| format!("error {error:#}\n"), |_| "ok\n".into()));
            });
            return;
        }
        // Every change, in the log: what turned a wallpaper off is then known.
        tracing::info!("wallpaper: {}", match &command { Command::Set { path } => format!("set {}", path.display()), other => format!("{other:?}") });
        let generation = {
            let mut state = self.state.lock().unwrap();
            state.generation += 1;
            if let Some(worker) = state.worker.take() { worker.abort(); }
            state.computer = status.id.clone();
            if command == Command::Off {
                state.stop();
                state.status.plan = None;
                state.status.message = "Wallpaper is off".into();
                state.status.poster = None;
                state.progress = None;
                state.coordinator = None;
                state.report_to = None;
                state.status.computers.clear();
                state.authority_off = true;
                state.connections.clear();
                state.pending = None; state.awaiting.clear(); state.applied_picture = None; state.still_fallback = false;
                if let Some(inbox) = &state.inbox { inbox.forget_wallpaper(); }
            }
            state.generation
        };
        let runtime = self.clone();
        let mut state = self.state.lock().unwrap();
        let task = tokio::spawn(async move {
            let result = runtime.execute(&dir, status, command, peers, node, generation).await;
            if let Err(error) = &result {
                let mut state = runtime.state.lock().unwrap();
                if state.generation == generation { state.status.message = error.to_string(); state.progress = None; }
            }
            let _ = reply.send(result.map_or_else(|error| format!("error {error:#}\n"), |()| "ok\n".into()));
            runtime.finish_worker(generation);
        });
        state.worker = Some(task);
    }

    async fn execute(&self, dir: &Path, status: Status, command: Command, peers: Vec<Peer>, node: Node, generation: u64) -> Result<()> {
        let mut preparation_notice = None;
        if command == Command::Off {
            for peer in peers { send(&peer.control, &Wire::Off); }
            let (runtime, root) = (self.clone(), cache(dir).join("pictures"));
            let restored = tokio::task::spawn_blocking(move || {
                let _work = runtime.picture_work.lock().unwrap();
                ensure!(runtime.state.lock().unwrap().generation == generation, "Wallpaper selection changed");
                restore_picture(&root)
            }).await?;
            let state = self.state.lock().unwrap();
            ensure!(state.generation == generation, "Wallpaper selection changed");
            let saved = Saved { status: state.status.clone(), coordinator: None, previous: state.previous.clone(), authority_off: true };
            save_selection(dir, &saved)?;
            return restored;
        }
        let mut plan = self.state.lock().unwrap().status.plan.clone();
        if let Command::Set { path } = command {
            check_source(&path)?;
            let picture = one_mk_wallpaper::picture::is_picture(&path);
            // Refuse unsupported video before preparation can launch a decoder,
            // and a picture on a desktop whose wallpaper oneMK can't set.
            if !picture { one_mk_wallpaper::availability().map_err(anyhow::Error::msg)?; }
            #[cfg(target_os = "linux")]
            ensure!(!picture || one_mk_wallpaper::picture::available(), "{}", one_mk_wallpaper::picture::UNSUPPORTED_DESKTOP);
            let dir_owned = dir.to_owned();
            let title = path.file_name().context("Choose a video file")?.to_string_lossy().chars().take(64).collect::<String>();
            let mut canvas = one_mk_ctl::wallpaper::canvas(&status);
            if let Some(old) = plan.as_ref() {
                let mut reserved = old.canvas.clone();
                reserved.monitors.retain(|monitor| monitor.computer == status.id || status.peers.iter().any(|peer| peer.id == monitor.computer));
                reserved.update(canvas.monitors);
                canvas = reserved;
            }
            self.state.lock().unwrap().status.message = if picture { "Preparing picture…" } else { "Preparing video…" }.into();
            let prepared = if picture { None } else { match self.prepare(dir, &path, &canvas, generation).await {
                Ok(prepared) => prepared,
                Err(error) => { tracing::info!("Wallpaper preparation: {error}"); preparation_notice = Some("Could not prepare a smaller copy. Using the original video."); None }
            } };
            let path = prepared.as_ref().map_or(path, |prepared| prepared.0.clone());
            let (hash, source) = tokio::task::spawn_blocking(move || import(&dir_owned, &path)).await??;
            let inspected = if picture {
                let poster = cache(dir).join(format!("{hash}.png"));
                tokio::task::spawn_blocking(move || one_mk_wallpaper::picture::inspect(&helper()?, &source, &poster)).await?
            } else { inspect(dir, &hash).await };
            let media = match inspected {
                Ok(media) => media,
                Err(error) => {
                    // A file that can't be used is not kept until the next
                    // one that can: only what is chosen (or was just before) stays.
                    let state = self.state.lock().unwrap();
                    let mut keep = BTreeSet::new();
                    keep.extend(state.status.plan.iter().flat_map(|plan| [plan.hash.clone()].into_iter().chain(plan.asset_hash.clone())));
                    keep.extend(state.previous.clone());
                    discard_unused(dir, &hash, &keep);
                    return Err(error);
                }
            };
            let computers = std::iter::once(status.id.clone()).chain(status.peers.iter().map(|peer| peer.id.clone())).collect();
            plan = Some(Plan { hash, title, media, canvas, start_ms: now_ms() + 2000, fit: Default::default(), computers, pause_on_battery: true, paused: false, picture, asset_hash: None, spread: Default::default() });
        } else {
            let plan = plan.as_mut().context("Choose a video first")?;
            match command {
                Command::Pause { paused } => plan.paused = paused,
                Command::Fit { fit } => plan.fit = fit,
                Command::Spread { spread } => plan.spread = spread,
                Command::Battery { enabled } => plan.pause_on_battery = enabled,
                Command::Computer { id, enabled } => {
                    ensure!(id == status.id || status.peers.iter().any(|peer| peer.id == id), "Computer is no longer paired");
                    plan.computers.retain(|computer| computer != &id);
                    if enabled { plan.computers.push(id); }
                }
                _ => {}
            }
        }
        let plan = plan.context("Choose a video first")?;
        self.commit(dir, plan.clone(), status.id, None, generation).await?;
        let mut failures: Vec<_> = status.peers.iter().filter(|peer| peer.online && plan.computers.contains(&peer.id) && !peers.iter().any(|connected| connected.id == peer.id))
            .map(|peer| format!("Update oneMK on {} to play video wallpapers", peer.name)).collect();
        for peer in peers {
            ensure!(self.state.lock().unwrap().generation == generation, "Wallpaper selection changed");
            if !plan.computers.contains(&peer.id) { send(&peer.control, &Wire::Off); continue; }
            if let Err(error) = self.distribute(dir, &plan, &peer, &node, generation).await { failures.push(format!("{}: {error}", peer.name)); }
        }
        ensure!(failures.is_empty(), "{}", failures.join("; "));
        if let Some(notice) = preparation_notice { self.state.lock().unwrap().status.message = notice.into(); }
        Ok(())
    }

    async fn distribute(&self, dir: &Path, plan: &Plan, peer: &Peer, node: &Node, generation: u64) -> Result<()> {
        if !plan.computers.contains(&peer.id) { send(&peer.control, &Wire::Off); return Ok(()); }
        self.state.lock().unwrap().status.computers.insert(peer.id.clone(), Playback::Preparing);
        let connection = tokio::time::timeout(Duration::from_secs(5), node.connect_pinned(peer.address, peer.fingerprint)).await??;
        let mut delivered = plan.clone();
        if plan.picture {
            let (dir, plan, computer) = (dir.to_owned(), plan.clone(), peer.id.clone());
            delivered.asset_hash = Some(tokio::task::spawn_blocking(move || picture_asset(&dir, &plan, &computer)).await??);
        }
        let hash = delivered.asset_hash.as_ref().unwrap_or(&plan.hash);
        let path = cache_path(&cache(dir), hash)?;
        let file = crate::transfer::Outgoing { rel: format!("{hash}.video"), size: std::fs::metadata(&path)?.len(), path, mtime: 0 };
        self.state.lock().unwrap().awaiting.insert((peer.fingerprint, hash.clone()));
        send(&peer.control, &Wire::Plan { plan: delivered.clone() });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let wake = self.state.lock().unwrap().clock_wake.clone();
                let notified = wake.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                {
                    let state = self.state.lock().unwrap();
                    ensure!(state.generation == generation, "Wallpaper selection changed");
                    if !state.awaiting.contains(&(peer.fingerprint, hash.clone())) { return Ok(()); }
                }
                notified.await;
            }
        }).await.context("Update oneMK on the receiving computer before sharing this wallpaper")??;
        let progress = Arc::new(crate::transfer::Progress::default());
        self.state.lock().unwrap().progress = Some((peer.name.clone(), progress.clone()));
        let results = tokio::time::timeout(Duration::from_secs(600), crate::transfer::send_files_kind(&connection.conn, vec![file], progress, crate::transfer::new_batch_id(), StreamKind::WallpaperVideo)).await?;
        connection.conn.close(0u32.into(), b"wallpaper copied");
        ensure!(results.len() == 1 && matches!(results[0].1, crate::transfer::Outcome::Saved { .. }), "Could not copy the video to {}", peer.name);
        let mut state = self.state.lock().unwrap();
        ensure!(state.generation == generation, "Wallpaper selection changed");
        state.progress = None;
        send(&peer.control, &Wire::Plan { plan: delivered });
        Ok(())
    }

    pub fn receive(&self, dir: PathBuf, computer: String, sender: Fingerprint, control: ControlWriter, wire: Wire) {
        match wire {
            Wire::PlanReady { hash } => {
                let mut state = self.state.lock().unwrap();
                if state.awaiting.remove(&(sender, hash)) { state.clock_wake.notify_waiters(); }
            }
            Wire::Playback { hash, playback } => {
                let mut state = self.state.lock().unwrap();
                if state.coordinator.is_none() && state.status.plan.as_ref().is_some_and(|plan| plan.hash == hash) {
                    state.status.computers.insert(sender.short(), playback);
                }
            }
            Wire::Clock { sent_ms } => { let received_ms = now_ms(); send(&control, &Wire::ClockReply { sent_ms, received_ms, replied_ms: now_ms() }); }
            Wire::ClockReply { sent_ms, received_ms, replied_ms } => {
                let mut state = self.state.lock().unwrap();
                if state.coordinator != Some(sender) || state.clocks.remove(&sender) != Some(sent_ms) { return; }
                state.clock_wake.notify_waiters();
                if let Some((offset, rtt)) = clock_offset(sent_ms, received_ms, replied_ms, now_ms()) && state.best_rtt.is_none_or(|best| rtt <= best) {
                    state.offset = offset; state.best_rtt = Some(rtt);
                    state.clock_wake.notify_waiters();
                    if let Some(child) = &mut state.player && let Some(input) = child.stdin.as_mut() { let _ = writeln!(input, "offset {offset}"); }
                }
            }
            Wire::Off => {
                let mut state = self.state.lock().unwrap();
                if state.coordinator != Some(sender) { return; }
                tracing::info!("wallpaper: off, by the computer that chose it ({sender})");
                if let Some(worker) = state.worker.take() { worker.abort(); }
                state.generation += 1; state.stop(); state.status.plan = None; state.status.poster = None; state.status.message = "Wallpaper is off".into();
                let runtime = self.clone();
                let root = cache(&dir).join("pictures");
                let generation = state.generation;
                tokio::task::spawn_blocking(move || {
                    let _work = runtime.picture_work.lock().unwrap();
                    if runtime.state.lock().unwrap().generation != generation { return; }
                    let result = restore_picture(&root);
                    if let Err(error) = result { let mut state = runtime.state.lock().unwrap(); if state.generation == generation { state.status.message = error.to_string(); } }
                });
                state.pending = None; state.applied_picture = None; state.still_fallback = false;
                if let Some(inbox) = &state.inbox { inbox.forget_wallpaper(); }
                state.status.computers.clear();
                state.progress = None;
                state.authority_off = false;
                let saved = Saved { status: state.status.clone(), coordinator: Some(sender.to_string()), previous: state.previous.clone(), authority_off: false };
                if let Err(error) = save_selection(&dir, &saved) { tracing::warn!("Couldn't save the wallpaper selection: {error}"); }
            }
            Wire::Plan { plan } => {
                if !plan.valid() { return; }
                let inbox = self.inbox(&dir);
                let hash = plan.asset_hash.as_ref().unwrap_or(&plan.hash).clone();
                let generation = {
                    let mut state = self.state.lock().unwrap();
                    let current = state.status.plan.as_ref().map(|plan| plan.asset_hash.as_ref().unwrap_or(&plan.hash).as_str());
                    inbox.expect_wallpaper(sender, current, &hash);
                    send(&control, &Wire::PlanReady { hash: hash.clone() });
                    if state.pending.as_ref().is_some_and(|(peer, pending)| *peer == sender && same_playback_plan(pending, &plan)) { return; }
                    if state.coordinator == Some(sender) && state.status.plan.as_ref().is_some_and(|active| same_playback_plan(active, &plan)) && (state.player.is_some() || state.still_fallback || state.picture_mode == one_mk_wallpaper::picture::Mode::Native && state.applied_picture.is_some()) {
                        state.report_to = Some(control.clone());
                        if let Some(playback) = state.status.computers.get(&computer) { send(&control, &Wire::Playback { hash: plan.hash.clone(), playback: playback.clone() }); }
                        return;
                    }
                    if let Some(worker) = state.worker.take() { worker.abort(); }
                    state.generation += 1; state.deferred = None; state.report_to = Some(control.clone());
                    state.pending = Some((sender, plan.clone()));
                    if state.coordinator != Some(sender) { state.offset = 0; state.best_rtt = None; state.clocks.clear(); }
                    state.coordinator = Some(sender);
                    state.generation
                };
                let runtime = self.clone();
                let mut state = self.state.lock().unwrap();
                let task = tokio::spawn(async move {
                    let hash = plan.asset_hash.as_ref().unwrap_or(&plan.hash).clone();
                    let selection_hash = plan.hash.clone();
                    let picture = plan.picture;
                    let check_dir = dir.clone();
                    let wait_path = cache_path(&cache(&dir), &hash).expect("validated plan hash");
                    let received = tokio::time::timeout(Duration::from_secs(600), async {
                        loop {
                            let notified = inbox.wallpaper_changed.notified();
                            tokio::pin!(notified);
                            notified.as_mut().enable();
                            if tokio::fs::try_exists(&wait_path).await? { return anyhow::Ok(()); }
                            notified.await;
                        }
                    }).await;
                    let sampled = if !picture && matches!(received, Ok(Ok(()))) { runtime.sample_clock(sender, &control, generation).await } else { Ok(()) };
                    let check = tokio::task::spawn_blocking(move || -> Result<()> {
                        received.context("The wallpaper transfer timed out")??;
                        sampled?;
                        ensure!(digest(&cache_path(&cache(&check_dir), &hash)?)? == hash, "The received video is incomplete"); Ok(())
                    }).await;
                    let result = match check {
                        Ok(Ok(())) if picture => runtime.commit(&dir, plan, computer, Some(sender), generation).await,
                        Ok(Ok(())) => match inspect(&dir, &plan.hash).await {
                            Ok(_) => runtime.commit(&dir, plan, computer, Some(sender), generation).await,
                            Err(error) => Err(error),
                        },
                        Ok(Err(error)) => Err(error), Err(error) => Err(error.into()),
                    };
                    if let Err(error) = result {
                        let mut state = runtime.state.lock().unwrap();
                        if state.generation == generation { state.pending = None; state.status.message = error.to_string(); send(&control, &Wire::Playback { hash: selection_hash, playback: Playback::Error(error.to_string()) }); }
                        return;
                    }
                    { let mut state = runtime.state.lock().unwrap(); if state.generation != generation { return; } state.pending = None; }
                    if picture {
                        let playback = { let state = runtime.state.lock().unwrap(); state.status.computers.get(&state.computer).cloned().unwrap_or(Playback::Preparing) };
                        send(&control, &Wire::Playback { hash: selection_hash, playback });
                        runtime.finish_worker(generation); return;
                    }
                    let weak = Arc::downgrade(&runtime.state);
                    drop(runtime);
                    loop {
                        let (active, clock_wake) = {
                            let Some(shared) = weak.upgrade() else { return };
                            let state = shared.lock().unwrap();
                            if state.generation != generation { return; }
                            (state.player.is_some() && state.status.computers.get(&state.computer) == Some(&Playback::Ready), state.clock_wake.clone())
                        };
                        if !active { clock_wake.notified().await; continue; }
                        for _ in 0..5 {
                            let sent_ms = now_ms();
                            {
                                let Some(shared) = weak.upgrade() else { return };
                                let mut state = shared.lock().unwrap();
                                if state.generation != generation { return; }
                                if state.status.computers.get(&state.computer) != Some(&Playback::Ready) { break; }
                                state.clocks.insert(sender, sent_ms);
                            }
                            send(&control, &Wire::Clock { sent_ms });
                            tokio::select! { _ = tokio::time::sleep(Duration::from_millis(250)) => {}, _ = clock_wake.notified() => { break; } }
                        }
                        if let Some(shared) = weak.upgrade() {
                            let state = shared.lock().unwrap();
                            if state.status.computers.get(&state.computer) != Some(&Playback::Ready) { continue; }
                        }
                        tokio::select! { _ = tokio::time::sleep(Duration::from_secs(30)) => {}, _ = clock_wake.notified() => {} }
                        let Some(shared) = weak.upgrade() else { return };
                        shared.lock().unwrap().best_rtt = None;
                    }
                });
                state.worker = Some(task);
            }
        }
    }

    async fn sample_clock(&self, sender: Fingerprint, control: &ControlWriter, generation: u64) -> Result<()> {
        self.state.lock().unwrap().best_rtt = None;
        for _ in 0..5 {
            let sent_ms = now_ms();
            {
                let mut state = self.state.lock().unwrap();
                ensure!(state.generation == generation, "Wallpaper selection changed");
                state.clocks.insert(sender, sent_ms);
            }
            send(control, &Wire::Clock { sent_ms });
            let wake = self.state.lock().unwrap().clock_wake.clone();
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let notified = wake.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();
                    { let state = self.state.lock().unwrap(); ensure!(state.generation == generation, "Wallpaper selection changed"); if state.clocks.get(&sender) != Some(&sent_ms) { return anyhow::Ok(()); } }
                    notified.await;
                }
            }).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        ensure!(self.state.lock().unwrap().best_rtt.is_some(), "Could not synchronize the wallpaper clock before playback. Try again.");
        Ok(())
    }
}

fn send(control: &ControlWriter, wire: &Wire) {
    if let Ok(json) = serde_json::to_string(wire) && json.len() <= 12000 { control.send(Control::LiveWallpaper(json)); }
}

#[derive(clap::Args)]
pub struct Args {
    /// Print the result as JSON ({"ok": true, "message": ...}), or
    /// {"ok": false, "error": ...} with a non-zero exit when it failed.
    #[arg(long, global = true)] json: bool,
    #[command(subcommand)] command: Action,
}
#[derive(clap::Subcommand)]
enum Action {
    /// Show a video or picture as the wallpaper across your screens and
    /// computers (sharing across computers is experimental).
    Set {
        /// A video (MP4, MOV, M4V, WebM, MKV) or picture (PNG, JPEG, WebP, HEIC).
        #[arg(value_name = "FILE")] file: PathBuf,
    },
    /// Turn the wallpaper off everywhere and put your own wallpaper back.
    Off,
    /// Say what is shown, where, and anything experimental here.
    Status,
    /// Pause the video on its current frame.
    Pause,
    /// Play a paused video again.
    Play,
    /// Set up oneMK's GNOME extension, which video needs on GNOME Wayland
    /// (experimental; GNOME 50).
    Extension { #[command(subcommand)] command: crate::wallpaper::extension::Command },
}

pub async fn command(dir: &Path, args: Args) -> Result<()> {
    let json = args.json;
    match run(dir, args.command).await {
        Ok(result) => {
            if json { println!("{result}"); }
            else {
                println!("{}", result["message"].as_str().unwrap_or_default());
                if let Some(note) = result["experimental"].as_str() { println!("{note}"); }
            }
            Ok(())
        }
        Err(error) => {
            if json { println!("{}", serde_json::json!({"ok": false, "error": format!("{error:#}")})); }
            Err(error)
        }
    }
}

/// What `one-mk wallpaper status` says when no oneMK answers: nothing plays
/// without it, and a saved choice comes back when it starts.
fn not_running(dir: &Path) -> String {
    let chosen = std::fs::read(cache(dir).join("selection.json")).ok().and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
        .filter(|saved| !saved.authority_off).and_then(|saved| saved.status.plan).map(|plan| plan.title);
    let comes_back = chosen.map_or_else(String::new, |title| format!(" It brings back \"{title}\" when it starts."));
    format!("Wallpaper is off: oneMK is not running.{comes_back} Start it with `one-mk run`, or make it start when you log in with `one-mk install`.")
}

const NOT_RUNNING: &str = "oneMK is not running. Start it with `one-mk run` (or open the oneMK window), then try again.";

async fn run(dir: &Path, action: Action) -> Result<serde_json::Value> {
    if let Action::Extension { command } = action { return crate::wallpaper::extension::command(&command).await; }
    let request = match action {
        Action::Status => crate::control::Request::Status,
        action => crate::control::Request::LiveWallpaper(serde_json::to_string(&match action {
            Action::Set { file } => { check_source(&file)?; Command::Set { path: file.canonicalize().with_context(|| format!("Cannot open {}", file.display()))? } }
            Action::Off => Command::Off,
            Action::Pause => Command::Pause { paused: true }, Action::Play => Command::Pause { paused: false }, Action::Status | Action::Extension { .. } => unreachable!(),
        })?),
    };
    let asked_status = matches!(request, crate::control::Request::Status);
    let Some(mut reply) = crate::control::request(dir, request).await? else {
        if asked_status { return Ok(serde_json::json!({"ok": true, "running": false, "message": not_running(dir), "wallpaper": null})); }
        bail!(NOT_RUNNING);
    };
    if reply.starts_with("error ") { bail!("{}", reply.trim_start_matches("error ").trim()); }
    // A change answers with an acknowledgement, not the wallpaper's state:
    // what it says now comes from a status request.
    // Each computer says within moments whether it plays: until then the
    // count would include one that can't, and `status` would then disagree.
    if !asked_status {
        for attempt in 0..12 {
            reply = crate::control::request(dir, crate::control::Request::Status).await?.context(NOT_RUNNING)?;
            let heard = Status::decode(&reply).and_then(|status| status.wallpaper).is_none_or(|wallpaper| wallpaper.plan.as_ref().is_none_or(|plan|
                plan.canvas.monitors.iter().filter(|monitor| monitor.online && plan.computers.contains(&monitor.computer))
                    .all(|monitor| wallpaper.computers.contains_key(&monitor.computer))));
            if heard || attempt == 11 { break; }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    Ok(match Status::decode(&reply) {
        Some(status) => {
            let wallpaper = status.wallpaper.unwrap_or_default();
            let message = if wallpaper.plan.is_some() { wallpaper.message.clone() } else { "Wallpaper is off".into() };
            serde_json::json!({"ok": true, "running": true, "message": message, "experimental": wallpaper.experimental, "wallpaper": wallpaper})
        }
        None => serde_json::json!({"ok": true, "running": true, "message": "Wallpaper updated."}),
    })
}

/// `one-mk uninstall`: the wallpaper goes off and the person's own picture
/// comes back. A running oneMK does it (and tells the other computers), and
/// only its explicit "ok" counts. Only when no oneMK runs is it done here,
/// holding the daemon's own [`InstanceLock`](crate::control::InstanceLock)
/// so no daemon can start an apply meanwhile, and the saved choice is
/// marked off, so a later start doesn't bring it back. A oneMK that is
/// running but does not answer is an error, never a reason to restore
/// beside it.
pub async fn turn_off_for_uninstall(dir: &Path) -> Result<()> {
    let request = crate::control::Request::LiveWallpaper(serde_json::to_string(&Command::Off)?);
    match crate::control::request(dir, request).await {
        Ok(Some(reply)) => return confirmed_off(&reply),
        Ok(None) => {}
        Err(error) => return Err(error.context("could not ask the running oneMK to turn its wallpaper off; stop it with `one-mk stop` and run this again")),
    }
    let Some(_instance) = crate::control::InstanceLock::try_acquire(dir)? else {
        bail!("oneMK is starting or running for {} but did not take the request; stop it with `one-mk stop` and run this again", dir.display());
    };
    // It may have started (and stopped) between the request and the lock.
    let selection = cache(dir).join("selection.json");
    if let Some(mut saved) = std::fs::read(&selection).ok().and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok()) && saved.status.plan.is_some() {
        saved.status = one_mk_wallpaper::Status { message: "Wallpaper is off".into(), ..Default::default() };
        saved.coordinator = None;
        saved.authority_off = true;
        save_selection(dir, &saved)?;
    }
    restore_picture(&cache(dir).join("pictures"))
}

/// The running oneMK's answer to "wallpaper off": only "ok" is success. An
/// empty answer is what it says when it ran out of time.
fn confirmed_off(reply: &str) -> Result<()> {
    let reply = reply.trim();
    if let Some(error) = reply.strip_prefix("error ") {
        bail!("{}", error.trim());
    }
    ensure!(reply == "ok", "the running oneMK did not confirm the wallpaper is off{}; run `one-mk wallpaper off` and check again",
        if reply.is_empty() { " (it did not finish in time)".to_owned() } else { format!(" (it said {reply:?})") });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    fn plan() -> Plan {
        Plan { hash: "a".repeat(64), title: "test".into(), media: Media { width: 640, height: 360, duration_ms: 10000, frame_rate: 30 },
            canvas: one_mk_wallpaper::Canvas { monitors: ["host", "remote"].into_iter().enumerate().map(|(index, computer)| one_mk_wallpaper::Monitor {
                computer: computer.into(), name: computer.into(), rect: one_mk_wallpaper::Rect { x: index as f64 * 640.0, y: 0.0, width: 640.0, height: 360.0 }, local: None, scale: 1.0, online: true }).collect() },
            start_ms: now_ms(), fit: Default::default(), computers: vec!["host".into(), "remote".into()], pause_on_battery: true, paused: false, picture: false, asset_hash: None, spread: Default::default() }
    }

    #[cfg(unix)]
    #[test]
    fn still_player_ignores_pause_reports_its_mode_and_stops_when_deselected() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::default();
        let child = std::process::Command::new("sh").args(["-c", "while read line; do [ \"$line\" = quit ] && exit; done"]).stdin(Stdio::piped()).spawn().unwrap();
        let pid = child.id();
        {
            let mut state = runtime.state.lock().unwrap();
            let mut picture = plan(); picture.picture = true; picture.paused = true;
            state.status.plan = Some(picture);
            state.computer = "host".into();
            state.picture_mode = one_mk_wallpaper::picture::Mode::Overlay;
            state.player = Some(child);
            state.status.computers.insert("host".into(), Playback::Picture);
            Runtime::start(directory.path(), &mut state).unwrap();
            assert_eq!(state.player.as_ref().unwrap().id(), pid);
        }
        assert!(runtime.status().message.contains("your own wallpaper returns"));
        {
            let mut state = runtime.state.lock().unwrap();
            state.status.plan.as_mut().unwrap().computers.retain(|computer| computer != "host");
            Runtime::start(directory.path(), &mut state).unwrap();
            assert!(state.player.is_none());
            assert!(!state.status.computers.contains_key("host"));
        }
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    }

    async fn controls() -> (ControlWriter, quinn::RecvStream, one_mk_net::Peer, Node, Node) {
        let node = || Node::bind(Arc::new(one_mk_net::Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let (local, remote) = (node(), node());
        let (connected, accepted) = tokio::join!(local.connect(remote.local_addr().unwrap()), remote.accept());
        let connected = connected.unwrap(); let accepted = accepted.unwrap().unwrap();
        let (outgoing, _) = one_mk_net::open_stream(&connected.conn, StreamKind::Control).await.unwrap();
        let (_, _, incoming) = accepted.accept_stream().await.unwrap();
        (ControlWriter::spawn(outgoing), incoming, connected, local, remote)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reconnects_and_duplicate_plans_preserve_the_running_player() {
        let (control, mut incoming, peer, node, _remote) = controls().await;
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::default();
        let plan = plan();
        let child = std::process::Command::new("sleep").arg("60").spawn().unwrap();
        let pid = child.id();
        { let mut state = runtime.state.lock().unwrap(); state.status.plan = Some(plan.clone()); state.player = Some(child); state.computer = "host".into(); state.status.computers.insert("host".into(), Playback::Ready); }
        let layout = || Layout { computer: "host".into(), canvas: one_mk_wallpaper::Canvas { monitors: vec![plan.canvas.monitors[0].clone()] }, paired: BTreeSet::from(["remote".into()]) };
        runtime.reconcile(directory.path().to_owned(), layout(), Vec::new(), node.clone());
        assert!(runtime.state.lock().unwrap().worker.is_none());
        runtime.reconcile(directory.path().to_owned(), layout(), vec![Peer { id: "remote".into(), name: "remote".into(), fingerprint: peer.fingerprint, address: "127.0.0.1:9".parse().unwrap(), control: control.clone() }], node.clone());
        { let state = runtime.state.lock().unwrap(); assert_eq!(state.generation, 0); assert_eq!(state.player.as_ref().unwrap().id(), pid); assert!(state.worker.is_some()); }
        { let mut state = runtime.state.lock().unwrap(); state.worker.take().unwrap().abort(); state.coordinator = Some(peer.fingerprint); }
        let mut duplicate = plan.clone(); duplicate.canvas.monitors[1].online = false;
        runtime.receive(directory.path().to_owned(), "host".into(), peer.fingerprint, control, Wire::Plan { plan: duplicate });
        { let state = runtime.state.lock().unwrap();
            assert!(state.worker.is_none()); assert!(state.pending.is_none()); assert_eq!(state.generation, 0); assert_eq!(state.launch, 0);
            assert_eq!(state.player.as_ref().unwrap().id(), pid);
        }
        assert!(!cache_path(&cache(directory.path()), &plan.hash).unwrap().exists());
        let mut bytes = Vec::new();
        for expected in [Wire::PlanReady { hash: plan.hash.clone() }, Wire::Playback { hash: plan.hash.clone(), playback: Playback::Ready }] {
            let Some(Control::LiveWallpaper(json)) = one_mk_net::read_frame(&mut incoming, &mut bytes).await.unwrap() else { panic!("missing duplicate-plan acknowledgment") };
            assert_eq!(json, serde_json::to_string(&expected).unwrap());
        }
    }

    #[tokio::test]
    async fn initial_clock_samples_finish_before_any_player_starts() {
        let (control, mut incoming, peer, _local, _remote) = controls().await;
        let runtime = Runtime::default();
        runtime.state.lock().unwrap().coordinator = Some(peer.fingerprint);
        let replies = async {
            let mut bytes = Vec::new();
            for _ in 0..5 {
                let Some(Control::LiveWallpaper(json)) = one_mk_net::read_frame(&mut incoming, &mut bytes).await.unwrap() else { panic!("missing clock request") };
                let Wire::Clock { sent_ms } = serde_json::from_str(&json).unwrap() else { panic!("playback was sent before clock sampling") };
                assert!(runtime.state.lock().unwrap().player.is_none());
                let remote_time = now_ms() + 20000;
                runtime.receive(PathBuf::new(), "host".into(), peer.fingerprint, control.clone(), Wire::ClockReply { sent_ms, received_ms: remote_time, replied_ms: remote_time });
            }
        };
        let (sampled, ()) = tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(runtime.sample_clock(peer.fingerprint, &control, 0), replies) }).await.unwrap();
        sampled.unwrap();
        let state = runtime.state.lock().unwrap();
        assert!(state.best_rtt.is_some()); assert!((state.offset - 20000).abs() < 500); assert!(state.player.is_none());
    }

    #[test]
    fn cached_hash_is_invalidated_by_replacement_and_write() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("media");
        std::fs::write(&path, b"first").unwrap(); let first = digest(&path).unwrap();
        std::fs::write(&path, b"second").unwrap(); assert_ne!(digest(&path).unwrap(), first);
        let replacement = directory.path().join("replacement"); std::fs::write(&replacement, b"third").unwrap();
        std::fs::rename(replacement, &path).unwrap(); assert_ne!(digest(&path).unwrap(), first);
    }

    #[cfg(unix)]
    #[test]
    fn exclusive_presentation_reaps_previous_and_retiring_players_before_replacement() {
        let previous = std::process::Command::new("sleep").arg("60").spawn().unwrap();
        let retiring = std::process::Command::new("sleep").arg("60").spawn().unwrap();
        let pids = [previous.id(), retiring.id()];
        let mut state = State::default();
        state.player = Some(previous); state.retiring = vec![retiring]; state.computer = "host".into();
        state.status.computers.insert("host".into(), Playback::Ready);
        state.replace_player(true);
        assert!(state.player.is_none() && state.retiring.is_empty());
        assert_eq!(state.launch, 1);
        for pid in pids { assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1); }
    }

    #[cfg(unix)]
    #[test]
    fn replacement_keeps_old_player_until_ready_and_ignores_stopped_launches() {
        let runtime = Runtime::default();
        let previous = std::process::Command::new("sleep").arg("60").spawn().unwrap();
        let previous_pid = previous.id();
        let current = std::process::Command::new("sh").args(["-c", "read trigger; printf '%s\\n' '{\"state\":\"ready\"}'; exec sleep 60"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        {
            let mut state = runtime.state.lock().unwrap();
            state.retiring.push(previous);
            state.player = Some(current);
            state.launch = 1;
            state.computer = "host".into();
        }
        runtime.listen();
        assert_eq!(unsafe { libc::kill(previous_pid as i32, 0) }, 0);
        {
            let mut state = runtime.state.lock().unwrap();
            state.generation += 1;
            writeln!(state.player.as_mut().unwrap().stdin.as_mut().unwrap(), "ready").unwrap();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while runtime.state.lock().unwrap().status.computers.get("host") != Some(&Playback::Ready) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(unsafe { libc::kill(previous_pid as i32, 0) }, -1);
        runtime.shutdown();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(runtime.state.lock().unwrap().status.computers.get("host"), Some(&Playback::Ready));
    }

    #[test]
    fn a_file_the_helper_cannot_read_is_said_plainly_without_rusts_error_prefix() {
        assert_eq!(probe_failure("Error: The system could not decode this video. Check that GStreamer has a decoder for it.\n"),
            "The system could not decode this video. Check that GStreamer has a decoder for it.");
        assert_eq!(probe_failure("Error: Cannot open it\n\nCaused by:\n    0: no such file\n"), "Cannot open it");
        assert_eq!(probe_failure("already plain"), "already plain");
        assert!(probe_failure("  \n").contains("could not read this file"));
        assert!(!probe_failure("Error:").starts_with("Error"));
    }

    #[test]
    fn an_unusable_file_is_not_kept_but_the_chosen_and_previous_ones_are() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(cache(directory.path())).unwrap();
        let [bad, chosen, before] = ["b", "c", "d"].map(|letter| letter.repeat(64));
        for hash in [&bad, &chosen, &before] {
            std::fs::write(cache_path(&cache(directory.path()), hash).unwrap(), b"x").unwrap();
            std::fs::write(cache(directory.path()).join(format!("{hash}.png")), b"x").unwrap();
        }
        let keep = BTreeSet::from([chosen.clone(), before.clone()]);
        discard_unused(directory.path(), &bad, &keep);
        assert!(!cache_path(&cache(directory.path()), &bad).unwrap().exists() && !cache(directory.path()).join(format!("{bad}.png")).exists());
        // Choosing the file that already plays, and failing, must not delete it.
        discard_unused(directory.path(), &chosen, &keep);
        discard_unused(directory.path(), &before, &keep);
        assert!(cache_path(&cache(directory.path()), &chosen).unwrap().exists() && cache_path(&cache(directory.path()), &before).unwrap().exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_wallpaper_set_while_locked_or_asleep_says_that_is_why_it_is_paused() {
        for (locked, asleep) in [(true, false), (false, true)] {
            let mut state = State::default();
            state.computer = "host".into();
            state.status.plan = Some(plan());
            state.locked = locked; state.asleep = asleep;
            Runtime::start(Path::new("/nonexistent-one-mk-test"), &mut state).unwrap();
            assert!(state.status.message.contains("locked or asleep") && state.status.message.contains("unlock"), "{}", state.status.message);
            assert_eq!(state.status.computers.get("host"), Some(&Playback::Paused(HELD_MESSAGE.into())));
        }
        // Paused by the person (not locked): still just "Paused".
        let mut state = State::default();
        state.computer = "host".into();
        state.status.plan = Some(Plan { paused: true, ..plan() });
        Runtime::start(Path::new("/nonexistent-one-mk-test"), &mut state).unwrap();
        assert_eq!(state.status.message, "Paused");
    }

    #[cfg(unix)]
    #[test]
    fn status_without_a_running_service_names_the_saved_choice() {
        let directory = tempfile::tempdir().unwrap();
        assert!(not_running(directory.path()).starts_with("Wallpaper is off: oneMK is not running.") && !not_running(directory.path()).contains("brings back"));
        let saved = Saved { status: one_mk_wallpaper::Status { plan: Some(Plan { title: "beach.mp4".into(), ..plan() }), ..Default::default() }, coordinator: None, previous: None, authority_off: false };
        save_selection(directory.path(), &saved).unwrap();
        assert!(not_running(directory.path()).contains("brings back \"beach.mp4\" when it starts"));
        let saved = Saved { authority_off: true, ..saved };
        save_selection(directory.path(), &saved).unwrap();
        assert!(!not_running(directory.path()).contains("brings back"));
    }

    #[test]
    fn a_missing_file_or_a_folder_is_named_and_explained() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("holiday.mp4");
        let error = check_source(&missing).unwrap_err().to_string();
        assert!(error.contains("holiday.mp4") && error.contains("does not exist"), "{error}");
        let error = check_source(directory.path()).unwrap_err().to_string();
        assert!(error.contains("is a folder") && !error.contains("changed before sending"), "{error}");
        std::fs::write(&missing, b"video").unwrap();
        check_source(&missing).unwrap();
    }

    #[test]
    fn a_still_shown_instead_of_a_video_is_said_with_the_one_step_fix() {
        let error = with_still_note(anyhow::anyhow!(one_mk_wallpaper::GNOME_PLAYBACK_UNAVAILABLE), true).to_string();
        assert!(error.contains("one-mk wallpaper extension install") && error.contains("first frame as a still picture"), "{error}");
        assert_eq!(with_still_note(anyhow::anyhow!("plain"), false).to_string(), "plain");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uninstall_without_a_running_daemon_marks_the_choice_off() {
        let directory = tempfile::tempdir().unwrap();
        let saved = Saved { status: one_mk_wallpaper::Status { plan: Some(plan()), message: "Playing".into(), ..Default::default() }, coordinator: None, previous: None, authority_off: false };
        save_selection(directory.path(), &saved).unwrap();
        turn_off_for_uninstall(directory.path()).await.unwrap();
        let after: Saved = serde_json::from_slice(&std::fs::read(cache(directory.path()).join("selection.json")).unwrap()).unwrap();
        assert!(after.status.plan.is_none() && after.authority_off);
        // Nothing chosen at all: nothing to do, and no error.
        turn_off_for_uninstall(tempfile::tempdir().unwrap().path()).await.unwrap();
    }

    /// A oneMK that holds the folder (running, starting, or not answering)
    /// is never raced: nothing is restored or rewritten beside it.
    #[cfg(unix)]
    #[tokio::test]
    async fn uninstall_never_restores_beside_a_daemon_that_holds_the_folder() {
        let directory = tempfile::tempdir().unwrap();
        let saved = Saved { status: one_mk_wallpaper::Status { plan: Some(plan()), message: "Playing".into(), ..Default::default() }, coordinator: None, previous: None, authority_off: false };
        save_selection(directory.path(), &saved).unwrap();
        let before = std::fs::read(cache(directory.path()).join("selection.json")).unwrap();
        let held = crate::control::InstanceLock::try_acquire(directory.path()).unwrap().unwrap();
        let error = format!("{:#}", turn_off_for_uninstall(directory.path()).await.unwrap_err());
        assert!(error.contains("one-mk stop"), "{error}");
        assert_eq!(std::fs::read(cache(directory.path()).join("selection.json")).unwrap(), before, "the saved choice is untouched");
        drop(held);
        // A process another test is starting at this moment can still hold the lock's open file
        // between its fork and its exec (the descriptor is close-on-exec): give it a moment.
        let mut result = turn_off_for_uninstall(directory.path()).await;
        for _ in 0..40 {
            if result.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            result = turn_off_for_uninstall(directory.path()).await;
        }
        result.unwrap();
    }

    /// Only an explicit "ok" from the running oneMK is success.
    #[test]
    fn only_an_explicit_ok_confirms_the_wallpaper_is_off() {
        confirmed_off("ok\n").unwrap();
        for reply in ["", "\n", "error Wallpaper is not available\n", "maybe\n"] {
            assert!(confirmed_off(reply).is_err(), "{reply:?}");
        }
        assert!(confirmed_off("").unwrap_err().to_string().contains("did not finish in time"));
    }

    #[cfg(unix)]
    #[test]
    fn decoder_error_is_not_replaced_by_generic_exit_message() {
        let runtime = Runtime::default();
        let child = std::process::Command::new("sh").args(["-c", "printf '%s\\n' '{\"state\":\"error\",\"message\":\"Decoder unavailable\"}'"]).stdout(Stdio::piped()).spawn().unwrap();
        {
            let mut state = runtime.state.lock().unwrap();
            state.player = Some(child); state.computer = "host".into();
        }
        runtime.listen();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while runtime.state.lock().unwrap().player.is_some() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(runtime.state.lock().unwrap().status.computers.get("host"), Some(&Playback::Error("Decoder unavailable".into())));
    }
    #[test]
    fn off_intent_survives_restart_without_starting_a_player() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(cache(directory.path())).unwrap();
        let saved = Saved { status: one_mk_wallpaper::Status { message: "Wallpaper is off".into(), ..Default::default() }, coordinator: None, previous: None, authority_off: true };
        std::fs::write(cache(directory.path()).join("selection.json"), serde_json::to_vec(&saved).unwrap()).unwrap();
        let runtime = Runtime::default();
        runtime.restore(directory.path(), "computer".into());
        let state = runtime.state.lock().unwrap();
        assert!(state.authority_off);
        assert!(state.player.is_none());
        assert!(state.status.plan.is_none());
        assert_eq!(state.status.message, "Wallpaper is off");
    }

    #[test]
    fn finished_copy_clears_progress_but_not_newer_work() {
        let runtime = Runtime::default();
        runtime.state.lock().unwrap().progress = Some(("PC".into(), Arc::new(crate::transfer::Progress::default())));
        runtime.finish_worker(1);
        assert!(runtime.state.lock().unwrap().progress.is_some());
        runtime.finish_worker(0);
        assert!(runtime.state.lock().unwrap().progress.is_none());
    }

    #[test]
    fn hash_import_is_content_addressed_and_keeps_two_videos() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("video.mp4");
        std::fs::write(&source, b"first video").unwrap();
        let (first, first_path) = import(directory.path(), &source).unwrap();
        assert_eq!(import(directory.path(), &source).unwrap().1, first_path);
        std::fs::write(&first_path, b"corrupted cache").unwrap();
        import(directory.path(), &source).unwrap();
        assert_eq!(std::fs::read(&first_path).unwrap(), b"first video");
        std::fs::write(&source, b"second video").unwrap();
        let (second, second_path) = import(directory.path(), &source).unwrap();
        std::fs::write(&source, b"third video").unwrap();
        let (third, third_path) = import(directory.path(), &source).unwrap();
        let links = one_mk_wallpaper::links_dir(&cache(directory.path()));
        std::fs::create_dir_all(&links).unwrap();
        std::fs::hard_link(&first_path, links.join(format!("{first}.mp4"))).unwrap();
        std::fs::hard_link(&third_path, links.join(format!("{third}.mov"))).unwrap();
        prune_cache(&cache(directory.path()), &third, Some(&second)).unwrap();
        assert!(!first_path.exists()); assert!(second_path.exists() && third_path.exists()); assert_ne!(first, second);
        // The pruned video's link goes too (it would keep its bytes on disk).
        assert!(!links.join(format!("{first}.mp4")).exists() && links.join(format!("{third}.mov")).exists());
    }
    #[cfg(unix)]
    #[test]
    fn off_and_drop_reap_the_owned_player() {
        for picture in [false, true] {
            let runtime = Runtime::default();
            let child = std::process::Command::new("sleep").arg("60").stdin(Stdio::piped()).spawn().unwrap();
            let pid = child.id();
            {
                let mut state = runtime.state.lock().unwrap();
                let mut plan = plan(); plan.picture = picture;
                state.status.plan = Some(plan);
                state.picture_mode = one_mk_wallpaper::picture::Mode::Overlay;
                state.player = Some(child);
            }
            runtime.shutdown();
            assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
            assert!(runtime.state.lock().unwrap().player.is_none());
        }
    }

    #[cfg(unix)]
    #[test]
    fn lock_sleep_and_last_owner_drop_reap_the_player() {
        for (picture, reason) in [false, true].into_iter().flat_map(|picture| ["lock", "sleep", "drop"].map(|reason| (picture, reason))) {
            let runtime = Runtime::default();
            let child = std::process::Command::new("sleep").arg("60").stdin(Stdio::piped()).spawn().unwrap();
            let pid = child.id();
            {
                let mut state = runtime.state.lock().unwrap();
                let mut plan = plan(); plan.picture = picture;
                state.status.plan = Some(plan);
                state.picture_mode = one_mk_wallpaper::picture::Mode::Overlay;
                state.player = Some(child);
            }
            match reason {
                "lock" => runtime.hold(Path::new("/tmp"), Some(true), None),
                "sleep" => runtime.hold(Path::new("/tmp"), None, Some(true)),
                _ => drop(runtime),
            }
            assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1, "{reason}");
        }
    }
}
