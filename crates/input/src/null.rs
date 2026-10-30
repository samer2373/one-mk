use crate::system::{FakeFocus, FakeLocker, FocusNotify};
use crate::{Backend, Bus, CaptureCmd, DeviceRef, Event, Fingerprint, Inject, InputDevice, Kinds};
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
    /// If set, locking fails with this message.
    pub lock_error: Arc<Mutex<Option<String>>>,
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
    let lock_error = Arc::new(Mutex::new(None));
    let backend = Backend {
        name: "null",
        events: ev_rx,
        capture: cap_tx,
        inject: inj_tx,
        focus: Box::new(FakeFocus { focus: focus.clone(), probes: focus_probes.clone(), notify: focus_notify.clone() }),
        locker: Box::new(FakeLocker { events: ev_tx.clone(), count: locks.clone(), error: lock_error.clone() }),
    };
    (backend, NullHandles { events: ev_tx, capture: cap_rx, inject: inj_rx, focus, focus_probes, focus_notify, locks, lock_error })
}

impl NullHandles {
    /// Says which keyboards and mice are connected, as a backend does at
    /// start and whenever one is plugged in, unplugged, sleeps or wakes.
    pub fn devices(&self, devices: Vec<InputDevice>) {
        let _ = self.events.send(Event::Devices(devices));
    }

    /// A key (or, for button codes, a button) from `device` (`None`: the
    /// backend can't tell which).
    pub fn key_from(&self, code: u32, down: bool, device: Option<DeviceRef>) {
        let ev = if one_mk_proto::keynames::is_button(code) {
            Event::Button { button: code, down, device }
        } else {
            Event::Key { code, down, device }
        };
        let _ = self.events.send(ev);
    }
}

/// A USB keyboard for tests: `DeviceRef(id)`, vendor 0x1234, product `id`.
pub fn fake_keyboard(id: u32, name: &str) -> InputDevice {
    InputDevice {
        id: DeviceRef(id),
        fingerprint: Fingerprint::new(Bus::Usb, 0x1234, id as u16, None, name),
        kinds: Kinds { keyboard: true, pointer: false },
    }
}
