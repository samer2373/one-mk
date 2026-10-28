use crate::{Backend, CaptureCmd, Event, Inject};
use one_mk_proto::Desktop;
use tokio::sync::mpsc;

/// Test-side ends of a null backend.
pub struct NullHandles {
    pub events: mpsc::UnboundedSender<Event>,
    pub capture: mpsc::UnboundedReceiver<CaptureCmd>,
    pub inject: mpsc::UnboundedReceiver<Inject>,
}

pub(crate) fn open(desktop: Desktop) -> (Backend, NullHandles) {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (cap_tx, cap_rx) = mpsc::unbounded_channel();
    let (inj_tx, inj_rx) = mpsc::unbounded_channel();
    let _ = ev_tx.send(Event::Desktop(desktop));
    (
        Backend { events: ev_rx, capture: cap_tx, inject: inj_tx },
        NullHandles { events: ev_tx, capture: cap_rx, inject: inj_rx },
    )
}
