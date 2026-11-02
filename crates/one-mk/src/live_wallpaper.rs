use anyhow::{Context, Result, bail, ensure};
use one_mk_ctl::status::Status;
use one_mk_net::{ControlWriter, Fingerprint, Node};
use one_mk_proto::{Control, StreamKind};
use one_mk_wallpaper::{Command, Media, Plan, PlayerConfig, Wire, cache_path, clock_offset, now_ms, prune_cache};
use ring::digest::{Context as Digest, SHA256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub struct Peer { pub id: String, pub name: String, pub fingerprint: Fingerprint, pub address: std::net::SocketAddr, pub control: ControlWriter }

#[derive(Default)]
struct State {
    status: one_mk_wallpaper::Status,
    player: Option<Child>,
    generation: u64,
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
    deferred: Option<(PathBuf, Status, Vec<Peer>, Node)>,
    inbox: Option<Arc<crate::transfer::Inbox>>,
    authority_off: bool,
}

impl State {
    fn stop(&mut self) {
        if let Some(mut child) = self.player.take() {
            if let Some(mut input) = child.stdin.take() { let _ = writeln!(input, "quit"); }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Drop for State { fn drop(&mut self) { if let Some(worker) = self.worker.take() { worker.abort(); } self.stop(); } }

#[derive(Clone, Default)]
pub struct Runtime { state: Arc<Mutex<State>> }

#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    status: one_mk_wallpaper::Status,
    coordinator: Option<String>,
    previous: Option<String>,
    #[serde(default)]
    authority_off: bool,
}

fn helper() -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    one_mk_ctl::install::sibling(&executable, "one-mk-wallpaper").context("The video wallpaper player is missing from this oneMK installation. Reinstall oneMK to add it.")
}

pub fn cache(dir: &Path) -> PathBuf { dir.join("wallpaper-video") }

pub(crate) fn digest(path: &Path) -> Result<String> {
    let mut file = crate::transfer::open_regular(path, std::fs::symlink_metadata(path)?.len())?;
    ensure!(file.metadata()?.is_file(), "Choose a regular video file");
    ensure!(file.metadata()?.len() <= one_mk_wallpaper::MAX_MEDIA_BYTES, "Choose a video smaller than 4 GB");
    let mut hash = Digest::new(&SHA256);
    let mut buffer = vec![0u8; 256 * 1024];
    loop { let count = file.read(&mut buffer)?; if count == 0 { break; } hash.update(&buffer[..count]); }
    Ok(hash.finish().as_ref().iter().map(|byte| format!("{byte:02x}")).collect())
}

fn import(dir: &Path, path: &Path) -> Result<(String, PathBuf)> {
    let root = cache(dir);
    std::fs::create_dir_all(&root)?;
    let hash = digest(path)?;
    let destination = cache_path(&root, &hash)?;
    if digest(&destination).ok().as_deref() != Some(hash.as_str()) {
        let temporary = root.join(format!("{}.part", crate::link::random_u64()));
        let result = (|| -> Result<()> {
            let mut source = crate::transfer::open_regular(path, std::fs::metadata(path)?.len())?;
            let mut target = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
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

async fn inspect(dir: &Path, hash: &str) -> Result<Media> {
    let mut probe = tokio::process::Command::new(helper()?);
    probe.arg("--probe").arg(cache_path(&cache(dir), hash)?).arg(cache(dir).join(format!("{hash}.png"))).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(40), probe.output()).await.context("The system took too long to read this video")??;
    ensure!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr).trim());
    Ok(serde_json::from_slice(&output.stdout)?)
}

impl Runtime {
    pub fn inbox(&self, dir: &Path) -> Arc<crate::transfer::Inbox> {
        self.state.lock().unwrap().inbox.get_or_insert_with(|| crate::transfer::Inbox::with_notifier(cache(dir), Arc::new(crate::transfer::Quiet))).clone()
    }

    pub fn restore(&self, dir: &Path, computer: String) {
        let Ok(bytes) = std::fs::read(cache(dir).join("selection.json")) else { return };
        let Ok(saved) = serde_json::from_slice::<Saved>(&bytes) else { return };
        if saved.status.plan.as_ref().is_some_and(|plan| !plan.valid()) { return; }
        let mut state = self.state.lock().unwrap();
        state.computer = computer;
        state.coordinator = saved.coordinator.and_then(|fingerprint| fingerprint.parse().ok());
        state.previous = saved.previous;
        state.authority_off = saved.authority_off;
        state.status = saved.status;
        if let Err(error) = Self::start(dir, &mut state) { state.status.message = error.to_string(); }
    }

    pub fn reconcile(&self, dir: PathBuf, status: Status, peers: Vec<Peer>, node: Node) {
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
            state.deferred = Some((dir, status, peers, node));
            return;
        }
        let Some(plan) = state.status.plan.as_ref() else { return };
        let mut updated = plan.clone();
        updated.canvas.monitors.retain(|monitor| monitor.computer == status.id || status.peers.iter().any(|peer| peer.id == monitor.computer));
        updated.computers.retain(|computer| computer == &status.id || status.peers.iter().any(|peer| &peer.id == computer));
        updated.canvas.update(one_mk_ctl::wallpaper::canvas(&status).monitors);
        let connections = peers.iter().map(|peer| peer.fingerprint).collect();
        if updated.canvas == plan.canvas && state.connections == connections { return; }
        state.connections = connections;
        state.generation += 1;
        let generation = state.generation;
        drop(state);
        let runtime = self.clone();
        let mut state = self.state.lock().unwrap();
        let task = tokio::spawn(async move {
            match runtime.commit(&dir, updated.clone(), status.id, None, generation) {
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
        if let Some((dir, status, peers, node)) = deferred { self.reconcile(dir, status, peers, node); }
    }

    pub fn status(&self) -> one_mk_wallpaper::Status {
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
        if state.player.is_some() && let Some(directory) = &state.directory
            && let Ok(message) = std::fs::read_to_string(cache(directory).join("playback.txt"))
            && message != "Playing" {
            status.message = message;
        }
        if let Some((name, progress)) = &state.progress {
            use std::sync::atomic::Ordering::Relaxed;
            let total = progress.total.load(Relaxed);
            let done = progress.done.load(Relaxed);
            status.message = format!("Copying to {name}… {}%", done.saturating_mul(100).checked_div(total).unwrap_or(0));
        }
        if let Err(reason) = one_mk_wallpaper::availability() {
            status.available = false;
            status.message = reason.into();
        } else { status.available = true; }
        status
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
        if state.locked || state.asleep { state.stop(); }
        else if let Err(error) = Self::start(dir, &mut state) { state.status.message = error.to_string(); }
    }

    fn start(dir: &Path, state: &mut State) -> Result<()> {
        state.directory = Some(dir.to_owned());
        let Some(plan) = state.status.plan.as_ref() else { return Ok(()) };
        if state.locked || state.asleep || plan.paused || !plan.computers.contains(&state.computer) {
            state.stop();
            state.status.message = "Paused".into();
            return Ok(());
        }
        if state.player.is_some() { return Ok(()); }
        one_mk_wallpaper::availability().map_err(anyhow::Error::msg)?;
        let config = PlayerConfig { plan: plan.clone(), computer: state.computer.clone(), path: cache_path(&cache(dir), &plan.hash)?, clock_offset_ms: state.offset };
        let config_path = cache(dir).join("player.json");
        let _ = std::fs::remove_file(cache(dir).join("playback.txt"));
        std::fs::write(&config_path, serde_json::to_vec(&config)?)?;
        let log = std::fs::File::create(cache(dir).join("player.log"))?;
        state.player = Some(std::process::Command::new(helper()?).arg(config_path).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(log).spawn()?);
        state.status.message = plan.playing_message();
        Ok(())
    }

    fn commit(&self, dir: &Path, plan: Plan, computer: String, coordinator: Option<Fingerprint>, generation: u64) -> Result<()> {
        ensure!(plan.valid(), "The wallpaper layout or video is invalid");
        ensure!(serde_json::to_vec(&Wire::Plan { plan: plan.clone() })?.len() <= 12000, "This wallpaper layout has too many monitors to share");
        let mut state = self.state.lock().unwrap();
        ensure!(state.generation == generation, "Wallpaper selection changed");
        state.stop();
        state.previous = state.status.plan.as_ref().map(|old| old.hash.clone()).filter(|hash| hash != &plan.hash).or(state.previous.take());
        state.computer = computer;
        if state.coordinator != coordinator { state.offset = 0; state.best_rtt = None; state.clocks.clear(); }
        state.coordinator = coordinator;
        state.authority_off = false;
        let hash = plan.hash.clone();
        state.status.plan = Some(plan);
        state.progress = None;
        state.status.poster = cache(dir).join(format!("{hash}.png")).exists().then(|| cache(dir).join(format!("{hash}.png")));
        let saved = Saved { status: state.status.clone(), coordinator: state.coordinator.map(|fingerprint| fingerprint.to_string()), previous: state.previous.clone(), authority_off: false };
        let temporary = cache(dir).join("selection.tmp");
        std::fs::write(&temporary, serde_json::to_vec(&saved)?)?;
        std::fs::rename(temporary, cache(dir).join("selection.json"))?;
        prune_cache(&cache(dir), &hash, state.previous.as_deref())?;
        Self::start(dir, &mut state)
    }

    pub fn request(&self, dir: PathBuf, status: Status, command: Command, peers: Vec<Peer>, node: Node, reply: tokio::sync::oneshot::Sender<String>) {
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
                state.authority_off = true;
                state.connections.clear();
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
        if command == Command::Off {
            for peer in peers { send(&peer.control, &Wire::Off); }
            let state = self.state.lock().unwrap();
            ensure!(state.generation == generation, "Wallpaper selection changed");
            let saved = Saved { status: state.status.clone(), coordinator: None, previous: state.previous.clone(), authority_off: true };
            std::fs::create_dir_all(cache(dir))?;
            let temporary = cache(dir).join("selection.tmp");
            std::fs::write(&temporary, serde_json::to_vec(&saved)?)?;
            std::fs::rename(temporary, cache(dir).join("selection.json"))?;
            return Ok(());
        }
        let mut plan = self.state.lock().unwrap().status.plan.clone();
        if let Command::Set { path } = command {
            let dir_owned = dir.to_owned();
            let title = path.file_name().context("Choose a video file")?.to_string_lossy().chars().take(64).collect::<String>();
            self.state.lock().unwrap().status.message = "Preparing video…".into();
            let (hash, _) = tokio::task::spawn_blocking(move || import(&dir_owned, &path)).await??;
            let media = inspect(dir, &hash).await?;
            let mut canvas = one_mk_ctl::wallpaper::canvas(&status);
            if let Some(old) = plan.as_ref() {
                let mut reserved = old.canvas.clone();
                reserved.monitors.retain(|monitor| monitor.computer == status.id || status.peers.iter().any(|peer| peer.id == monitor.computer));
                reserved.update(canvas.monitors);
                canvas = reserved;
            }
            let computers = std::iter::once(status.id.clone()).chain(status.peers.iter().map(|peer| peer.id.clone())).collect();
            plan = Some(Plan { hash, title, media, canvas, start_ms: now_ms() + 2000, fit: Default::default(), computers, pause_on_battery: true, paused: false });
        } else {
            let plan = plan.as_mut().context("Choose a video first")?;
            match command {
                Command::Pause { paused } => plan.paused = paused,
                Command::Fit { fit } => plan.fit = fit,
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
        self.commit(dir, plan.clone(), status.id, None, generation)?;
        let mut failures: Vec<_> = status.peers.iter().filter(|peer| peer.online && plan.computers.contains(&peer.id) && !peers.iter().any(|connected| connected.id == peer.id))
            .map(|peer| format!("Update oneMK on {} to play video wallpapers", peer.name)).collect();
        for peer in peers {
            ensure!(self.state.lock().unwrap().generation == generation, "Wallpaper selection changed");
            if !plan.computers.contains(&peer.id) { send(&peer.control, &Wire::Off); continue; }
            if let Err(error) = self.distribute(dir, &plan, &peer, &node, generation).await { failures.push(format!("{}: {error}", peer.name)); }
        }
        ensure!(failures.is_empty(), "{}", failures.join("; "));
        Ok(())
    }

    async fn distribute(&self, dir: &Path, plan: &Plan, peer: &Peer, node: &Node, generation: u64) -> Result<()> {
        if !plan.computers.contains(&peer.id) { send(&peer.control, &Wire::Off); return Ok(()); }
        let connection = tokio::time::timeout(Duration::from_secs(5), node.connect_pinned(peer.address, peer.fingerprint)).await??;
        let path = cache_path(&cache(dir), &plan.hash)?;
        let file = crate::transfer::Outgoing { rel: format!("{}.video", plan.hash), size: std::fs::metadata(&path)?.len(), path, mtime: 0 };
        let progress = Arc::new(crate::transfer::Progress::default());
        self.state.lock().unwrap().progress = Some((peer.name.clone(), progress.clone()));
        let results = tokio::time::timeout(Duration::from_secs(600), crate::transfer::send_files_kind(&connection.conn, vec![file], progress, crate::transfer::new_batch_id(), StreamKind::WallpaperVideo)).await?;
        connection.conn.close(0u32.into(), b"wallpaper copied");
        ensure!(results.len() == 1 && matches!(results[0].1, crate::transfer::Outcome::Saved { .. }), "Could not copy the video to {}", peer.name);
        let mut state = self.state.lock().unwrap();
        ensure!(state.generation == generation, "Wallpaper selection changed");
        state.progress = None;
        send(&peer.control, &Wire::Plan { plan: plan.clone() });
        Ok(())
    }

    pub fn receive(&self, dir: PathBuf, computer: String, sender: Fingerprint, control: ControlWriter, wire: Wire) {
        match wire {
            Wire::Clock { sent_ms } => { let received_ms = now_ms(); send(&control, &Wire::ClockReply { sent_ms, received_ms, replied_ms: now_ms() }); }
            Wire::ClockReply { sent_ms, received_ms, replied_ms } => {
                let mut state = self.state.lock().unwrap();
                if state.coordinator != Some(sender) || state.clocks.remove(&sender) != Some(sent_ms) { return; }
                if let Some((offset, rtt)) = clock_offset(sent_ms, received_ms, replied_ms, now_ms()) && state.best_rtt.is_none_or(|best| rtt <= best) {
                    state.offset = offset; state.best_rtt = Some(rtt);
                    if let Some(child) = &mut state.player && let Some(input) = child.stdin.as_mut() { let _ = writeln!(input, "offset {offset}"); }
                }
            }
            Wire::Off => {
                let mut state = self.state.lock().unwrap();
                if state.coordinator != Some(sender) { return; }
                if let Some(worker) = state.worker.take() { worker.abort(); }
                state.generation += 1; state.stop(); state.status.plan = None; state.status.poster = None; state.status.message = "Wallpaper is off".into();
                state.progress = None;
                state.authority_off = false;
                let saved = Saved { status: state.status.clone(), coordinator: Some(sender.to_string()), previous: state.previous.clone(), authority_off: false };
                let temporary = cache(&dir).join("selection.tmp");
                if let Ok(bytes) = serde_json::to_vec(&saved) && std::fs::write(&temporary, bytes).is_ok() { let _ = std::fs::rename(temporary, cache(&dir).join("selection.json")); }
            }
            Wire::Plan { plan } => {
                if !plan.valid() { return; }
                let generation = { let mut state = self.state.lock().unwrap(); if let Some(worker) = state.worker.take() { worker.abort(); } state.generation += 1; state.deferred = None; state.generation };
                let runtime = self.clone();
                let mut state = self.state.lock().unwrap();
                let task = tokio::spawn(async move {
                    let hash = plan.hash.clone();
                    let check_dir = dir.clone();
                    let check = tokio::task::spawn_blocking(move || -> Result<()> {
                        ensure!(digest(&cache_path(&cache(&check_dir), &hash)?)? == hash, "The received video is incomplete"); Ok(())
                    }).await;
                    let result = match check {
                        Ok(Ok(())) => match inspect(&dir, &plan.hash).await {
                            Ok(_) => runtime.commit(&dir, plan, computer, Some(sender), generation),
                            Err(error) => Err(error),
                        },
                        Ok(Err(error)) => Err(error), Err(error) => Err(error.into()),
                    };
                    if let Err(error) = result {
                        let mut state = runtime.state.lock().unwrap();
                        if state.generation == generation { state.status.message = error.to_string(); }
                        return;
                    }
                    let weak = Arc::downgrade(&runtime.state);
                    drop(runtime);
                    loop {
                        for _ in 0..5 {
                            let sent_ms = now_ms();
                            {
                                let Some(shared) = weak.upgrade() else { return };
                                let mut state = shared.lock().unwrap();
                                if state.generation != generation { return; }
                                state.clocks.insert(sender, sent_ms);
                            }
                            send(&control, &Wire::Clock { sent_ms });
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        }
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        let Some(shared) = weak.upgrade() else { return };
                        shared.lock().unwrap().best_rtt = None;
                    }
                });
                state.worker = Some(task);
            }
        }
    }
}

fn send(control: &ControlWriter, wire: &Wire) {
    if let Ok(json) = serde_json::to_string(wire) && json.len() <= 12000 { control.send(Control::LiveWallpaper(json)); }
}

#[derive(clap::Args)]
pub struct Args { #[command(subcommand)] command: Action }
#[derive(clap::Subcommand)]
enum Action { Set { file: PathBuf }, Off, Status, Pause, Play }

pub async fn command(dir: &Path, args: Args) -> Result<()> {
    let request = match args.command {
        Action::Status => crate::control::Request::Status,
        action => crate::control::Request::LiveWallpaper(serde_json::to_string(&match action {
            Action::Set { file } => Command::Set { path: file.canonicalize()? }, Action::Off => Command::Off,
            Action::Pause => Command::Pause { paused: true }, Action::Play => Command::Pause { paused: false }, Action::Status => unreachable!(),
        })?),
    };
    let reply = crate::control::request(dir, request).await?.context("Start oneMK first")?;
    if let Some(status) = Status::decode(&reply) { println!("{}", status.wallpaper.map_or_else(|| "Wallpaper is off".into(), |status| status.message)); }
    else if reply.starts_with("error ") { bail!("{}", reply.trim_start_matches("error ").trim()); }
    else { println!("Wallpaper updated."); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        prune_cache(&cache(directory.path()), &third, Some(&second)).unwrap();
        assert!(!first_path.exists()); assert!(second_path.exists() && third_path.exists()); assert_ne!(first, second);
    }
    #[cfg(unix)]
    #[test]
    fn off_and_drop_reap_the_owned_player() {
        let runtime = Runtime::default();
        let child = std::process::Command::new("sleep").arg("60").stdin(Stdio::piped()).spawn().unwrap();
        let pid = child.id();
        runtime.state.lock().unwrap().player = Some(child);
        runtime.shutdown();
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
        assert!(runtime.state.lock().unwrap().player.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn lock_sleep_and_last_owner_drop_reap_the_player() {
        for reason in ["lock", "sleep", "drop"] {
            let runtime = Runtime::default();
            let child = std::process::Command::new("sleep").arg("60").stdin(Stdio::piped()).spawn().unwrap();
            let pid = child.id();
            runtime.state.lock().unwrap().player = Some(child);
            match reason {
                "lock" => runtime.hold(Path::new("/tmp"), Some(true), None),
                "sleep" => runtime.hold(Path::new("/tmp"), None, Some(true)),
                _ => drop(runtime),
            }
            assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1, "{reason}");
        }
    }
}
