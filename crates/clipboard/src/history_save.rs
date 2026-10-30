use crate::history::{Entry, SAVE_PREFIX};
use anyhow::Result;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub(crate) struct Snapshot {
    pub path: Option<PathBuf>,
    pub entries: Vec<Entry>,
}

#[derive(Default)]
struct State {
    pending: Option<Snapshot>,
    remove: Vec<PathBuf>,
    submitted: u64,
    completed: u64,
    stopping: bool,
    error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct Saver(Arc<Handle>);

struct Handle {
    state: Arc<(Mutex<State>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        let (mutex, wake) = &*self.state;
        mutex.lock().unwrap_or_else(|error| error.into_inner()).stopping = true;
        wake.notify_all();
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

impl Saver {
    pub fn new() -> Self {
        let state = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker = state.clone();
        let thread = std::thread::Builder::new().name("one-mk-history-save".into()).spawn(move || {
            let (mutex, wake) = &*worker;
            loop {
                let mut state = mutex.lock().unwrap_or_else(|error| error.into_inner());
                while state.pending.is_none() && !state.stopping {
                    state = wake.wait(state).unwrap_or_else(|error| error.into_inner());
                }
                if state.pending.is_none() && state.stopping { return; }
                if !state.stopping {
                    let deadline = std::time::Instant::now() + Duration::from_millis(150);
                    while !state.stopping && std::time::Instant::now() < deadline {
                        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                        state = wake.wait_timeout(state, remaining).unwrap_or_else(|error| error.into_inner()).0;
                    }
                }
                let snapshot = state.pending.take().expect("pending save");
                let remove = std::mem::take(&mut state.remove);
                let sequence = state.submitted;
                drop(state);
                let result = (|| -> Result<()> {
                    for path in &remove { remove_file(path)?; }
                    save(snapshot)
                })();
                let mut state = mutex.lock().unwrap_or_else(|error| error.into_inner());
                if result.is_err() {
                    for path in remove { if !state.remove.contains(&path) { state.remove.push(path); } }
                }
                state.error = result.err().map(|error| format!("Could not save/remove clipboard history: {error}; check the settings folder and retry."));
                state.completed = sequence;
                wake.notify_all();
            }
        }).expect("spawn history save thread");
        Self(Arc::new(Handle { state, thread: Some(thread) }))
    }

    pub fn submit(&self, snapshot: Snapshot, remove: Option<PathBuf>) {
        let (mutex, wake) = &*self.0.state;
        let mut state = mutex.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(path) = remove && !state.remove.contains(&path) { state.remove.push(path); }
        state.pending = Some(snapshot);
        state.submitted += 1;
        wake.notify_all();
    }

    pub fn flush(&self) -> Result<()> {
        let (mutex, wake) = &*self.0.state;
        let mut state = mutex.lock().unwrap_or_else(|error| error.into_inner());
        let target = state.submitted;
        while state.completed < target {
            state = wake.wait(state).unwrap_or_else(|error| error.into_inner());
        }
        if let Some(error) = &state.error { anyhow::bail!("{error}"); }
        Ok(())
    }

    pub fn error(&self) -> Option<String> {
        self.0.state.0.try_lock().ok().and_then(|state| state.error.clone())
    }
}

fn remove_file(path: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {},
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    #[cfg(unix)]
    if let Some(directory) = path.parent() { std::fs::File::open(directory)?.sync_all()?; }
    Ok(())
}

fn save(snapshot: Snapshot) -> Result<()> {
    let Some(path) = snapshot.path else { return Ok(()); };
    if snapshot.entries.is_empty() { return remove_file(&path); }
    let directory = path.parent().ok_or_else(|| anyhow::anyhow!("missing settings folder"))?;
    let mut temporary = tempfile::Builder::new().prefix(SAVE_PREFIX).tempfile_in(directory)?;
    let file = temporary.as_file_mut();
    file.write_all(b"OMKH1")?;
    file.write_all(&(snapshot.entries.len() as u64).to_le_bytes())?;
    for entry in snapshot.entries {
        file.write_all(&entry.id.to_le_bytes())?;
        file.write_all(&entry.timestamp.to_le_bytes())?;
        super::history::write_field(file, Some(entry.origin.as_bytes()))?;
        super::history::write_field(file, entry.data.text.as_deref().map(str::as_bytes))?;
        super::history::write_field(file, entry.data.html.as_deref().map(str::as_bytes))?;
        super::history::write_field(file, entry.data.png.as_deref())?;
    }
    file.sync_all()?;
    temporary.persist(&path)?;
    #[cfg(unix)]
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
}
