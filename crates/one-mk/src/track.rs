//! What is being transferred right now, for `one-mk status` and the window,
//! and the one notification per received transfer.
//!
//! Receiving: the [`Inbox`](crate::transfer::Inbox) opens a [`FileTicket`]
//! per offered file and reports bytes through it. Files from one send are
//! grouped by the sender's [`Batch`] (all its streams repeat it), so there is
//! one progress line for a folder of 500 files and one notification at the end.
//! A send from an older version has no batch; then the files of a device are
//! grouped as they come.
//!
//! A transfer is over when every file of its batch has been settled, or,
//! when that never happens (the sender stopped, or it is an older one and
//! the end can't be known), once nothing has been in flight for
//! [`QUIET`]. That wait is the only timer, and it exists only between a
//! file finishing and the notification going out: nothing runs while idle.
//!
//! Sending: `one-mk send` is a separate process, so it reports its own
//! progress to the daemon over the control channel ([`Transfers::report`]);
//! the daemon forgets a report that stops coming.

use crate::notify::{FilesReceived, Notifier};
use one_mk_ctl::status::TransferStatus;
use one_mk_net::Fingerprint;
use one_mk_proto::Batch;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Without a known end, how long with nothing in flight before a transfer counts as over.
pub const QUIET: Duration = Duration::from_millis(2000);
/// A report from `one-mk send` this old is stale: that process is gone.
const REPORT_TTL: Duration = Duration::from_secs(5);
/// No bytes for this long reads as stalled (speed 0).
const STALL: Duration = Duration::from_secs(2);
/// Speed is measured over at least this long.
const RATE_WINDOW: Duration = Duration::from_millis(250);

pub struct Transfers {
    state: Mutex<State>,
    notifier: Arc<dyn Notifier>,
    notify: AtomicBool,
    folder: PathBuf,
    quiet: Duration,
}

#[derive(Default)]
struct State {
    rx: HashMap<(Fingerprint, u64), Rx>,
    tx: HashMap<u64, (TransferStatus, Instant)>,
    /// Sends that said they are finished, and when: a report that was
    /// still on its way then must not bring them back.
    ended: HashMap<u64, Instant>,
}

/// A smoothed bytes-per-second figure.
pub struct Rate {
    per_s: f64,
    since: Instant,
    bytes_then: u64,
    last_bytes_at: Instant,
}

impl Rate {
    pub fn new(now: Instant) -> Self {
        Self { per_s: 0.0, since: now, bytes_then: 0, last_bytes_at: now }
    }

    /// `total` bytes have now moved. A total that went backwards (two
    /// counters read a moment apart can do that) counts as no progress.
    pub fn update(&mut self, now: Instant, total: u64) {
        self.last_bytes_at = now;
        let dt = now.saturating_duration_since(self.since);
        if dt >= RATE_WINDOW {
            let instant = total.saturating_sub(self.bytes_then) as f64 / dt.as_secs_f64();
            self.per_s = if self.per_s == 0.0 { instant } else { 0.6 * self.per_s + 0.4 * instant };
            self.since = now;
            self.bytes_then = total;
        }
    }

    pub fn get(&self, now: Instant) -> u64 {
        if now.duration_since(self.last_bytes_at) > STALL { 0 } else { self.per_s as u64 }
    }
}

struct Rx {
    peer: String,
    /// What the sender said the whole send is (0: unknown).
    expect_files: u32,
    expect_bytes: u64,
    /// Files offered so far, and their bytes.
    offered_files: u32,
    offered_bytes: u64,
    /// Files finished one way or another.
    settled: u32,
    /// Bytes that are in place (including a resumed file's earlier part).
    done: u64,
    /// Bytes that actually crossed the network, for the speed.
    moved: u64,
    active: u32,
    saved: Vec<String>,
    saved_bytes: u64,
    rate: Rate,
    /// Changes whenever a file settles, so a stale quiet timer does nothing.
    generation: u64,
}

impl Rx {
    fn complete(&self) -> bool {
        self.active == 0 && self.expect_files > 0 && self.settled >= self.expect_files
    }

    fn status(&self, id: u64, now: Instant) -> TransferStatus {
        let known = self.expect_files > 0;
        TransferStatus {
            id,
            sending: false,
            peer: self.peer.clone(),
            files: if known { self.expect_files } else { self.offered_files },
            done: self.done,
            total: if known { self.expect_bytes.max(self.done) } else { self.offered_bytes.max(self.done) },
            rate: self.rate.get(now),
        }
    }
}

impl Transfers {
    pub fn new(folder: PathBuf, notifier: Arc<dyn Notifier>) -> Arc<Self> {
        Self::with_quiet(folder, notifier, QUIET)
    }

    pub fn with_quiet(folder: PathBuf, notifier: Arc<dyn Notifier>, quiet: Duration) -> Arc<Self> {
        Arc::new(Self { state: Mutex::default(), notifier, notify: AtomicBool::new(true), folder, quiet })
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether finished transfers raise a notification.
    pub fn set_notify(&self, on: bool) {
        self.notify.store(on, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub fn notifying(&self) -> bool {
        self.notify.load(Ordering::Relaxed)
    }

    /// A file of `size` bytes is being offered by `peer`.
    #[cfg(test)]
    pub fn begin(self: &Arc<Self>, fp: Fingerprint, peer: &str, batch: Batch, size: u64) -> FileTicket {
        self.begin_inner(fp, peer, batch, size, false).unwrap()
    }

    pub fn try_begin(self: &Arc<Self>, fp: Fingerprint, peer: &str, batch: Batch, size: u64) -> Result<FileTicket, &'static str> {
        self.begin_inner(fp, peer, batch, size, true)
    }

    fn begin_inner(self: &Arc<Self>, fp: Fingerprint, peer: &str, batch: Batch, size: u64, validate: bool) -> Result<FileTicket, &'static str> {
        let key = (fp, batch.id);
        let now = Instant::now();
        let mut s = self.state();
        if validate {
            const MAX_FILES: u32 = 10_000;
            const MAX_BYTES: u64 = 1 << 40;
            if size > MAX_BYTES || batch.bytes > MAX_BYTES || batch.files > MAX_FILES {
                return Err("file batch exceeds the limit of 10000 files or 1 TiB");
            }
            if let Some(rx) = s.rx.get(&key) {
                if batch.files != rx.expect_files || batch.bytes != rx.expect_bytes {
                    return Err("file batch metadata changed during transfer");
                }
                if rx.offered_files >= MAX_FILES || rx.offered_bytes.saturating_add(size) > MAX_BYTES
                    || (batch.files > 0 && rx.offered_files >= batch.files)
                    || (batch.bytes > 0 && rx.offered_bytes.saturating_add(size) > batch.bytes) {
                    return Err("file offers exceed the batch count or size");
                }
            } else if batch.bytes > 0 && size > batch.bytes {
                return Err("file size exceeds the declared batch size");
            }
        }
        let rx = s.rx.entry(key).or_insert_with(|| Rx {
            peer: peer.to_owned(),
            expect_files: batch.files,
            expect_bytes: batch.bytes,
            offered_files: 0,
            offered_bytes: 0,
            settled: 0,
            done: 0,
            moved: 0,
            active: 0,
            saved: Vec::new(),
            saved_bytes: 0,
            rate: Rate::new(now),
            generation: 0,
        });
        rx.offered_files = rx.offered_files.saturating_add(1);
        rx.offered_bytes = rx.offered_bytes.saturating_add(size);
        rx.active = rx.active.saturating_add(1);
        rx.generation = rx.generation.wrapping_add(1);
        drop(s);
        Ok(FileTicket { owner: self.clone(), key, size, credited: 0, settled: false })
    }

    /// Called when a file has ended: saved as `name`, or not (`None`).
    fn settle(self: &Arc<Self>, key: (Fingerprint, u64), size: u64, credited: u64, saved: Option<String>) {
        let mut s = self.state();
        let Some(rx) = s.rx.get_mut(&key) else { return };
        rx.active = rx.active.saturating_sub(1);
        rx.settled = rx.settled.saturating_add(1);
        rx.generation = rx.generation.wrapping_add(1);
        if let Some(name) = saved {
            // A file that arrived whole is all done, however its bytes were counted.
            rx.done = rx.done.saturating_add(size.saturating_sub(credited));
            rx.saved.push(name);
            rx.saved_bytes = rx.saved_bytes.saturating_add(size);
        }
        if rx.complete() {
            let rx = s.rx.remove(&key).expect("just found");
            drop(s);
            self.finish(rx);
        } else if rx.active == 0 {
            let generation = rx.generation;
            drop(s);
            self.expire_later(key, generation);
        }
    }

    /// Ends the transfer if nothing has happened to it for `quiet`.
    fn expire_later(self: &Arc<Self>, key: (Fingerprint, u64), generation: u64) {
        let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
        let weak = Arc::downgrade(self);
        let quiet = self.quiet;
        rt.spawn(async move {
            tokio::time::sleep(quiet).await;
            let Some(this) = weak.upgrade() else { return };
            let mut s = this.state();
            if s.rx.get(&key).is_some_and(|rx| rx.generation == generation && rx.active == 0) {
                let rx = s.rx.remove(&key).expect("just found");
                drop(s);
                this.finish(rx);
            }
        });
    }

    fn finish(&self, rx: Rx) {
        if rx.saved.is_empty() || !self.notify.load(Ordering::Relaxed) {
            return;
        }
        self.notifier.files_received(FilesReceived {
            peer: rx.peer,
            names: rx.saved,
            bytes: rx.saved_bytes,
            folder: self.folder.clone(),
        });
    }

    /// Progress of a send by a `one-mk send` process on this computer.
    pub fn report(&self, t: TransferStatus, finished: bool) {
        let now = Instant::now();
        let mut s = self.state();
        s.ended.retain(|_, at| now.duration_since(*at) < 2 * REPORT_TTL);
        if finished {
            s.tx.remove(&t.id);
            s.ended.insert(t.id, now);
        } else if !s.ended.contains_key(&t.id) {
            s.tx.insert(t.id, (TransferStatus { sending: true, ..t }, now));
        }
    }

    /// Transfers in flight now, for `status`.
    pub fn snapshot(&self) -> Vec<TransferStatus> {
        let now = Instant::now();
        let mut s = self.state();
        s.tx.retain(|_, (_, at)| now.duration_since(*at) < REPORT_TTL);
        let mut out: Vec<TransferStatus> = s.rx.iter().map(|((_, id), rx)| rx.status(*id, now)).collect();
        out.extend(s.tx.values().map(|(t, _)| t.clone()));
        out.sort_by(|a, b| (a.sending, &a.peer, a.id).cmp(&(b.sending, &b.peer, b.id)));
        out
    }
}

/// One offered file. Dropping it without [`saved`](Self::saved) counts the
/// file as not received.
pub struct FileTicket {
    owner: Arc<Transfers>,
    key: (Fingerprint, u64),
    size: u64,
    /// Bytes already added to the progress for this file.
    credited: u64,
    settled: bool,
}

impl FileTicket {
    /// The receiver already has the first `offset` bytes.
    pub fn resumed(&mut self, offset: u64) {
        self.advance(offset, false);
    }

    /// `n` more bytes arrived.
    pub fn bytes(&mut self, n: u64) {
        self.advance(n, true);
    }

    fn advance(&mut self, n: u64, over_network: bool) {
        let n = n.min(self.size - self.credited);
        self.credited += n;
        let now = Instant::now();
        if let Some(rx) = self.owner.state().rx.get_mut(&self.key) {
            rx.done = rx.done.saturating_add(n);
            if over_network {
                rx.moved = rx.moved.saturating_add(n);
                let moved = rx.moved;
                rx.rate.update(now, moved);
            }
        }
    }

    /// The file is verified and in place as `name`.
    pub fn saved(mut self, name: String) {
        self.settled = true;
        self.owner.settle(self.key, self.size, self.credited, Some(name));
    }
}

impl Drop for FileTicket {
    fn drop(&mut self) {
        if !self.settled {
            self.owner.settle(self.key, self.size, self.credited, None);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[tokio::test]
    async fn hostile_offer_totals_never_overflow() {
        let (transfers, _) = setup();
        let batch = batch(900, u32::MAX, u64::MAX);
        assert!(transfers.try_begin(FP, "peer", batch, u64::MAX).is_err());
        assert!(transfers.try_begin(FP, "peer", batch, 1).is_err());
        assert!(!transfers.state().rx.contains_key(&(FP, batch.id)));
    }

    #[tokio::test]
    async fn hostile_batches_are_refused_before_progress_accounting() {
        let (transfers, _) = setup();
        for files in [0, 1, 10_000, 10_001, u32::MAX] {
            for bytes in [0, 1, 1 << 40, (1 << 40) + 1, u64::MAX] {
                for size in [0, 1, 1 << 40, u64::MAX] {
                    let rejected = files > 10_000 || bytes > 1 << 40 || size > 1 << 40 || (bytes > 0 && size > bytes);
                    let result = transfers.try_begin(FP, "peer", batch(901, files, bytes), size);
                    assert_eq!(result.is_err(), rejected, "{files} {bytes} {size}");
                    if rejected { assert!(transfers.snapshot().is_empty()); }
                    transfers.state().rx.clear();
                    drop(result);
                }
            }
        }
        let first = transfers.try_begin(FP, "peer", batch(902, 2, 10), 8).unwrap();
        assert!(transfers.try_begin(FP, "peer", batch(902, 2, 10), 3).is_err());
        assert!(transfers.try_begin(FP, "peer", batch(902, 3, 10), 1).is_err());
        drop(first);
    }

    #[tokio::test]
    async fn undeclared_batches_cannot_accumulate_unlimited_offers() {
        let (transfers, _) = setup();
        let mut tickets = Vec::new();
        for _ in 0..10_000 {
            tickets.push(transfers.try_begin(FP, "peer", Batch::default(), 0).unwrap());
        }
        assert!(transfers.try_begin(FP, "peer", Batch::default(), 0).is_err());
        assert_eq!(transfers.state().rx[&(FP, 0)].offered_files, 10_000);
        drop(tickets);
    }

    /// Records what would have been shown.
    #[derive(Default)]
    pub struct Recorder(pub Mutex<Vec<FilesReceived>>);

    impl Notifier for Recorder {
        fn files_received(&self, n: FilesReceived) {
            self.0.lock().unwrap().push(n);
        }
    }

    impl Recorder {
        pub fn all(&self) -> Vec<FilesReceived> {
            self.0.lock().unwrap().clone()
        }
    }

    #[test]
    fn the_rate_survives_a_count_that_goes_backwards() {
        let t0 = Instant::now();
        let mut r = Rate::new(t0);
        let sec = Duration::from_secs(1);
        r.update(t0 + sec, 1000);
        assert_eq!(r.get(t0 + sec), 1000);
        // Fewer bytes than before: no progress, not an underflow or a huge rate.
        r.update(t0 + 2 * sec, 400);
        assert!(r.get(t0 + 2 * sec) <= 1000);
        // And counting resumes from the new figure.
        r.update(t0 + 3 * sec, 1400);
        assert!(r.get(t0 + 3 * sec) <= 1000, "{}", r.get(t0 + 3 * sec));
        // A time that goes backwards is ignored too.
        r.update(t0, 5);
    }

    const FP: Fingerprint = Fingerprint([1; 32]);
    const QUICK: Duration = Duration::from_millis(60);

    fn setup() -> (Arc<Transfers>, Arc<Recorder>) {
        let rec = Arc::new(Recorder::default());
        (Transfers::with_quiet(PathBuf::from("/dl"), rec.clone(), QUICK), rec)
    }

    fn batch(id: u64, files: u32, bytes: u64) -> Batch {
        Batch { id, files, bytes }
    }

    /// Receives a whole file as the inbox would.
    fn receive(t: &Arc<Transfers>, b: Batch, name: &str, size: u64) {
        let mut f = t.begin(FP, "desk", b, size);
        f.bytes(size);
        f.saved(name.to_owned());
    }

    #[tokio::test]
    async fn a_known_batch_gives_one_notification_when_its_last_file_lands() {
        let (t, rec) = setup();
        let b = batch(7, 3, 600);
        receive(&t, b, "a.txt", 100);
        receive(&t, b, "photos/b.jpg", 200);
        assert!(rec.all().is_empty(), "not over yet");
        receive(&t, b, "photos/c.jpg", 300);
        // At once, no waiting for a quiet period.
        assert_eq!(
            rec.all(),
            vec![FilesReceived {
                peer: "desk".into(),
                names: vec!["a.txt".into(), "photos/b.jpg".into(), "photos/c.jpg".into()],
                bytes: 600,
                folder: PathBuf::from("/dl"),
            }]
        );
        assert!(t.snapshot().is_empty(), "nothing left in flight");
        tokio::time::sleep(QUICK * 3).await;
        assert_eq!(rec.all().len(), 1, "and no second notification from a timer");
    }

    #[tokio::test]
    async fn files_in_parallel_streams_of_one_batch_are_one_transfer() {
        let (t, rec) = setup();
        let b = batch(9, 4, 400);
        let mut open: Vec<_> = (0..4).map(|i| t.begin(FP, "desk", b, 100 + i)).collect();
        assert_eq!(t.snapshot().len(), 1, "four streams, one line");
        let s = &t.snapshot()[0];
        assert_eq!((s.files, s.total, s.sending), (4, 400, false));
        for (i, f) in open.drain(..).enumerate() {
            f.saved(format!("f{i}"));
        }
        assert_eq!(rec.all().len(), 1);
        assert_eq!(rec.all()[0].names.len(), 4);
    }

    #[tokio::test]
    async fn an_old_sender_is_grouped_by_quiet_time() {
        let (t, rec) = setup();
        let none = Batch::default();
        receive(&t, none, "a", 10);
        receive(&t, none, "b", 10);
        assert!(rec.all().is_empty(), "can't know the end: waits to see if more come");
        // A third one arrives within the quiet period: still the same transfer.
        tokio::time::sleep(QUICK / 3).await;
        receive(&t, none, "c", 10);
        tokio::time::sleep(QUICK * 4).await;
        let got = rec.all();
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].names, ["a", "b", "c"]);
        // A later one is a new transfer, so a new notification.
        receive(&t, none, "d", 10);
        tokio::time::sleep(QUICK * 4).await;
        assert_eq!(rec.all().len(), 2);
    }

    #[tokio::test]
    async fn a_send_that_stops_early_still_reports_what_arrived() {
        let (t, rec) = setup();
        let b = batch(3, 10, 1000);
        receive(&t, b, "a", 100);
        receive(&t, b, "b", 100);
        assert_eq!(t.snapshot()[0].files, 10, "shows the whole send");
        tokio::time::sleep(QUICK * 4).await;
        assert_eq!(rec.all().len(), 1);
        assert_eq!(rec.all()[0].names, ["a", "b"]);
        assert!(t.snapshot().is_empty(), "and the line goes away");
    }

    #[tokio::test]
    async fn failures_count_toward_the_end_but_are_not_named() {
        let (t, rec) = setup();
        let b = batch(4, 3, 300);
        receive(&t, b, "ok1", 100);
        drop(t.begin(FP, "desk", b, 100)); // refused or failed
        receive(&t, b, "ok2", 100);
        let got = rec.all();
        assert_eq!(got.len(), 1, "the failed file doesn't hold the notification back");
        assert_eq!(got[0].names, ["ok1", "ok2"]);
        assert_eq!(got[0].bytes, 200);
    }

    #[tokio::test]
    async fn nothing_saved_means_no_notification() {
        let (t, rec) = setup();
        let b = batch(5, 1, 10);
        drop(t.begin(FP, "desk", b, 10));
        tokio::time::sleep(QUICK * 3).await;
        assert!(rec.all().is_empty());
        assert!(t.snapshot().is_empty());
    }

    #[tokio::test]
    async fn notifications_can_be_switched_off_live() {
        let (t, rec) = setup();
        t.set_notify(false);
        receive(&t, batch(1, 1, 5), "a", 5);
        assert!(rec.all().is_empty());
        t.set_notify(true);
        receive(&t, batch(2, 1, 5), "b", 5);
        assert_eq!(rec.all().len(), 1);
    }

    #[tokio::test]
    async fn two_senders_are_two_transfers() {
        let (t, rec) = setup();
        let other = Fingerprint([2; 32]);
        let (a, b) = (batch(1, 1, 5), batch(1, 1, 5)); // same id, different devices
        let ta = t.begin(FP, "desk", a, 5);
        let tb = t.begin(other, "laptop", b, 5);
        assert_eq!(t.snapshot().len(), 2);
        ta.saved("x".into());
        tb.saved("y".into());
        let peers: Vec<String> = rec.all().into_iter().map(|n| n.peer).collect();
        assert_eq!(peers, ["desk", "laptop"]);
    }

    #[tokio::test]
    async fn progress_counts_bytes_resumes_and_speed() {
        let (t, _rec) = setup();
        let mut f = t.begin(FP, "desk", batch(8, 2, 1000), 600);
        f.resumed(100);
        f.bytes(200);
        let s = t.snapshot().remove(0);
        assert_eq!((s.done, s.total, s.files, s.peer.as_str()), (300, 1000, 2, "desk"));
        assert_eq!(s.percent(), Some(30));
        // Speed appears once enough time has passed, and drops to 0 when it stalls.
        tokio::time::sleep(RATE_WINDOW + Duration::from_millis(20)).await;
        f.bytes(300);
        assert!(t.snapshot()[0].rate > 0);
        // More bytes than the file has can't push progress past it.
        f.bytes(10_000);
        assert_eq!(t.snapshot()[0].done, 600, "{:?}", t.snapshot());
        drop(f);
    }

    #[tokio::test]
    async fn a_file_that_arrived_whole_is_all_done() {
        let (t, _rec) = setup();
        let b = batch(6, 2, 20);
        let f = t.begin(FP, "desk", b, 10);
        f.saved("never counted bytes".into()); // e.g. a tiny file the receiver already had
        let s = t.snapshot().remove(0);
        assert_eq!(s.done, 10);
    }

    #[test]
    fn send_reports_are_shown_and_expire() {
        let (t, _) = setup();
        let r = TransferStatus { id: 11, sending: false, peer: "mac".into(), files: 5, done: 1, total: 9, rate: 2 };
        t.report(r.clone(), false);
        let s = t.snapshot();
        assert_eq!(s.len(), 1);
        assert!(s[0].sending, "reports are always for sends");
        t.report(TransferStatus { done: 5, ..r.clone() }, false);
        assert_eq!(t.snapshot()[0].done, 5, "updates replace");
        t.report(r.clone(), true);
        assert!(t.snapshot().is_empty(), "the sender said it is finished");
        t.report(TransferStatus { done: 8, ..r }, false);
        assert!(t.snapshot().is_empty(), "a late report does not bring a finished send back");

        // A sender that vanishes without saying so is dropped after a while.
        let gone = TransferStatus { id: 12, peer: "mac".into(), ..Default::default() };
        t.state().tx.insert(12, (gone, Instant::now() - REPORT_TTL - Duration::from_millis(1)));
        assert!(t.snapshot().is_empty());
    }

    #[test]
    fn speed_is_smoothed_and_stalls_to_zero() {
        let t0 = Instant::now();
        let mut r = Rate::new(t0);
        r.update(t0 + Duration::from_millis(100), 1_000_000);
        assert_eq!(r.get(t0 + Duration::from_millis(100)), 0, "too soon to say");
        r.update(t0 + Duration::from_secs(1), 10_000_000);
        assert_eq!(r.get(t0 + Duration::from_secs(1)), 10_000_000);
        r.update(t0 + Duration::from_secs(2), 12_000_000);
        // 0.6 * 10 MB/s + 0.4 * 2 MB/s
        assert_eq!(r.get(t0 + Duration::from_secs(2)), 6_800_000);
        assert_eq!(r.get(t0 + Duration::from_secs(5)), 0, "no bytes for a while");
    }
}
