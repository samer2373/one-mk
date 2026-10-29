use crate::system::{FakeFocus, FakeLocker, FocusNotify};
use crate::{Backend, CaptureCmd, Event, Inject};
use one_mk_proto::{Desktop, Focus};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// Test-side ends of a null backend.
pub struct NullHandles {
    pub events: mpsc::UnboundedSender<Event>,
    pub capture: mpsc::UnboundedReceiver<CaptureCmd>,
    pub inject: mpsc::UnboundedReceiver<Inject>,
    /// What the backend reports as the focused window (`None`: unknown).
    pub focus: Arc<Mutex<Option<Focus>>>,
    /// How many times the focused window was looked at.
    pub focus_probes: Arc<AtomicUsize>,
    /// Set while the daemon wants to hear that another window came to the
    /// front ([`crate::FocusProbe::watch`]); call it to say so.
    pub focus_notify: FocusNotify,
    /// How many times the screen was asked to lock. Each request is also
    /// reported back as `Event::ScreenLocked(true)`, as an OS would.
    pub locks: Arc<AtomicUsize>,
}

pub(crate) fn open(desktop: Desktop) -> (Backend, NullHandles) {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (cap_tx, cap_rx) = mpsc::unbounded_channel();
    let (inj_tx, inj_rx) = mpsc::unbounded_channel();
    let _ = ev_tx.send(Event::Desktop(desktop));
    let focus = Arc::new(Mutex::new(None));
    let focus_probes = Arc::new(AtomicUsize::new(0));
    let focus_notify: FocusNotify = Arc::new(Mutex::new(None));
    let locks = Arc::new(AtomicUsize::new(0));
    let backend = Backend {
        events: ev_rx,
        capture: cap_tx,
        inject: inj_tx,
        focus: Box::new(FakeFocus { focus: focus.clone(), probes: focus_probes.clone(), notify: focus_notify.clone() }),
        locker: Box::new(FakeLocker { events: ev_tx.clone(), count: locks.clone() }),
    };
    (backend, NullHandles { events: ev_tx, capture: cap_rx, inject: inj_rx, focus, focus_probes, focus_notify, locks })
}
