//! Broadcast (`one-mk broadcast`, see `docs/BROADCAST.md`): everything
//! typed, clicked and scrolled here goes to every connected computer at
//! once.
//!
//! Each computer gets a visit of its own (a [`Remote`]: its own pointer,
//! epoch, acknowledgement, queue, held keys, key mapper and Ctrl/Cmd swap),
//! entered as `one-mk switch` enters one, at the centre of its main screen,
//! but with no edge leading home. The single visit's code does the sending:
//! for each event, each visit in turn is put where that code looks for the
//! visit in progress (`App::remote`, empty while broadcasting) and taken
//! back after (see [`App::each_visit`]). A visit ends the way a single
//! visit ends (`App::end_visit`), so nothing stays held anywhere.

use super::tune::ScrollIn;
use super::{App, ENTER_TIMEOUT, IDLE, KEY_ESC, MAX_COALESCE, Remote, SWITCH_TIMEOUT};
use one_mk_ctl::client::Toggle;
use one_mk_input::{CaptureCmd, DeviceRef, Event};
use one_mk_net::Fingerprint;
use one_mk_proto::remap::Action;
use one_mk_proto::{Control, Datagram, Desktop, Modifier, Pos, RemoteCursor, Step};
use std::collections::VecDeque;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

/// After broadcast was stopped before capture started, a capture that
/// starts this late is still handed straight back.
const LATE_CAPTURE: Duration = Duration::from_secs(2);

pub(super) enum Broadcast {
    /// The backend was asked to capture, as `one-mk switch` asks it.
    Starting {
        until: Instant,
        /// The computer whose edge the backend pushes into (kept armed).
        edge: Fingerprint,
        /// Who asked, waiting for the answer (nobody for a shortcut).
        reply: Option<oneshot::Sender<String>>,
    },
    On(On),
    /// Stopped while starting: a capture that starts after all is handed
    /// straight back, until `until`.
    Cancelled { until: Instant },
}

pub(super) struct On {
    activation: u32,
    edge: Fingerprint,
    /// One visit per computer, in the order they were entered.
    visits: VecDeque<Remote>,
    /// Computers that let go by their idle rule, and where their pointer
    /// was: entered again with the next input.
    resting: Vec<(Fingerprint, Pos)>,
    last_input: Instant,
    /// Key mapper actions asked for during one event, run once after it.
    actions: Vec<Action>,
    /// A run of motion and the device of each, applied to every
    /// computer's pointer in turn (reused, so motion doesn't allocate).
    deltas: Vec<(f64, f64, Option<DeviceRef>)>,
}

/// Why broadcast ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum End {
    /// `one-mk broadcast off|toggle`, the shortcut, the tray or the window.
    Asked,
    /// `one-mk switch` or a switching shortcut.
    Switched,
    /// Ctrl+Alt+Shift+Esc.
    Emergency,
    ScreenLocked,
    /// No input for `[broadcast] idle_off_minutes`.
    Idle,
    /// Every computer disconnected or took control back.
    NoneLeft,
    /// The system ended this computer's input capture.
    CaptureLost,
    /// one-mk is stopping.
    Quit,
}

impl End {
    /// Why, as the end of "Broadcast stopped: ...".
    fn why(self, idle: Option<Duration>) -> String {
        match self {
            End::Asked => "you stopped it".into(),
            End::Switched => "you switched computers".into(),
            End::Emergency => "Ctrl+Alt+Shift+Esc".into(),
            End::ScreenLocked => "this computer's screen locked".into(),
            End::Idle => match idle.map(|d| d.as_secs() / 60) {
                Some(1) => "no input for 1 minute".into(),
                Some(n) => format!("no input for {n} minutes"),
                None => "no input".into(),
            },
            End::NoneLeft => "no computer is left (they disconnected or took control back)".into(),
            End::CaptureLost => "this computer's input capture ended".into(),
            End::Quit => "one-mk is stopping".into(),
        }
    }
}

/// How a visit leaves the broadcast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Out {
    /// It is told the visit is over, after its key mapper lets go.
    Leave,
    /// Its link is gone: there is nobody to tell (it releases by itself
    /// when its lease runs out).
    Gone,
    /// It let go by its idle rule: entered again with the next input.
    Rest,
}

/// Applies a run of motion to one computer's pointer, each delta tuned for
/// that computer and clamped to its screens in turn (deltas can't simply be
/// summed: the pointer stops at edges). The last position reached, if it
/// moved: the only one worth sending.
fn advance(
    cursor: &mut RemoteCursor,
    tune: impl Fn(f64, f64, Option<DeviceRef>) -> (f64, f64),
    deltas: &[(f64, f64, Option<DeviceRef>)],
) -> Option<Datagram> {
    let mut moved = None;
    for &(dx, dy, device) in deltas {
        let (dx, dy) = tune(dx, dy, device);
        // There are no exits, so `Leave` can't happen.
        if let Step::Move(d) = cursor.motion(dx, dy) {
            moved = Some(d);
        }
    }
    moved
}

/// A pointer for a broadcast to `desktop`, which sits on our `side`: at
/// `at`, or the centre of its main screen. No edge leads home.
fn cursor_for(desktop: Desktop, side: one_mk_proto::Side, at: Option<Pos>, epoch: u16) -> RemoteCursor {
    let pos = at.unwrap_or_else(|| desktop.primary().center());
    RemoteCursor::jump_to(desktop, side.opposite(), pos, Some(Vec::new()), epoch)
}

/// "A", "A and B", "A, B and C".
fn joined(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// "1 computer", "3 computers".
fn computers(n: usize) -> String {
    if n == 1 { "1 computer".into() } else { format!("{n} computers") }
}

/// The notification when broadcast starts.
fn started_text(names: &[String], hotkey: &str) -> (String, String) {
    let stop = if hotkey.is_empty() { "`one-mk broadcast off` stops it.".to_owned() } else { format!("{hotkey} stops it.") };
    (
        format!("Broadcasting to {}", computers(names.len())),
        format!("Everything you type, click and scroll goes to {}. {stop}", joined(names)),
    )
}

/// The control reply: `ok`/`already`, then `on` and the computers, or `off`.
fn reply_text(word: &str, names: &[String]) -> String {
    if names.is_empty() { format!("{word}\noff\n") } else { format!("{word}\non\n{}\n", names.join("\t")) }
}

fn answer(reply: Option<oneshot::Sender<String>>, text: String) {
    if let Some(r) = reply {
        let _ = r.send(text);
    }
}

impl App {
    fn on_ref(&self) -> Option<&On> {
        match &self.broadcast {
            Some(Broadcast::On(on)) => Some(on),
            _ => None,
        }
    }

    fn on_mut(&mut self) -> Option<&mut On> {
        match &mut self.broadcast {
            Some(Broadcast::On(on)) => Some(on),
            _ => None,
        }
    }

    /// Broadcast is on or starting.
    pub(super) fn broadcasting(&self) -> bool {
        matches!(self.broadcast, Some(Broadcast::On(_) | Broadcast::Starting { .. }))
    }

    /// The capture broadcast runs on (shortcuts are matched on its keys).
    pub(super) fn broadcast_activation(&self) -> Option<u32> {
        self.on_ref().map(|on| on.activation)
    }

    /// The computer whose edge stays armed while broadcast starts and runs,
    /// even with the pointer locked: the backend starts capturing there.
    pub(super) fn broadcast_edge(&self) -> Option<Fingerprint> {
        match &self.broadcast {
            Some(Broadcast::On(on)) => Some(on.edge),
            Some(Broadcast::Starting { edge, .. }) => Some(*edge),
            _ => None,
        }
    }

    /// The computers broadcast goes to, by name (for `one-mk status`).
    pub(super) fn broadcast_names(&self) -> Vec<String> {
        let Some(on) = self.on_ref() else { return Vec::new() };
        let fps = on.visits.iter().map(|r| r.peer).chain(on.resting.iter().map(|(fp, _)| *fp));
        let mut names: Vec<String> = fps.map(|fp| self.name_for(fp)).collect();
        names.sort_by_key(|n| n.to_lowercase());
        names
    }

    /// A computer's name as `one-mk status` and `devices` show it: the one
    /// it was paired under.
    pub(super) fn name_for(&self, fp: Fingerprint) -> String {
        match (self.trust.get(&fp), self.peers.get(&fp)) {
            (Some(d), _) => one_mk_proto::sanitize_name(&d.name),
            (None, Some(p)) => p.name.clone(),
            (None, None) => fp.short(),
        }
    }

    /// The shortcut that starts and stops broadcast, for people; empty if
    /// there is none.
    pub(super) fn broadcast_hotkey(&self) -> String {
        let h = &self.settings.hotkeys;
        let chord = h.bindings().ok().and_then(|b| b.iter().find(|(a, _)| *a == one_mk_input::hotkeys::Action::Broadcast));
        chord.map(|(_, c)| c.label()).unwrap_or_default()
    }

    /// How long broadcast may go without input.
    fn broadcast_idle_off(&self) -> Option<Duration> {
        #[cfg(test)]
        if let Some(d) = testing::get(&self.dir).0 {
            return d;
        }
        self.settings.broadcast.idle_off()
    }

    /// How long a controller may send nothing before this computer lets go
    /// of it (shorter in the daemon tests).
    pub(super) fn owner_idle(&self) -> Duration {
        #[cfg(test)]
        if let Some(d) = testing::get(&self.dir).1 {
            return d;
        }
        IDLE
    }

    /// Connected computers that said what their screens are, by name.
    fn broadcast_targets(&self) -> Vec<Fingerprint> {
        let mut t: Vec<(String, Fingerprint)> =
            self.peers.iter().filter(|(_, p)| p.desktop.is_some()).map(|(fp, _)| (self.name_for(*fp).to_lowercase(), *fp)).collect();
        t.sort();
        t.into_iter().map(|(_, fp)| fp).collect()
    }

    // --------------------------------------------------------- start, stop

    /// `one-mk broadcast on|off|toggle`, the shortcut, the tray and the
    /// window. Answered once broadcast has started (or could not).
    pub(super) fn broadcast_request(&mut self, t: Toggle, reply: oneshot::Sender<String>) {
        let on = self.broadcasting();
        if t.apply(on) == on {
            return answer(Some(reply), reply_text("already", &self.broadcast_names()));
        }
        if on {
            self.stop_broadcast(End::Asked);
            return answer(Some(reply), reply_text("ok", &[]));
        }
        self.start_broadcast(reply);
    }

    fn start_broadcast(&mut self, reply: oneshot::Sender<String>) {
        let targets = self.broadcast_targets();
        let Some(&edge) = targets.first() else {
            return answer(Some(reply), self.no_targets());
        };
        if self.lock_sync.is_locked() {
            return answer(Some(reply), "error this computer's screen is locked; unlock it first\n".into());
        }
        // The person here wants this computer's mouse and keyboard: whoever
        // controls this computer lets go first (as for `one-mk switch`).
        self.end_owner(true);
        self.switching = None;
        let captured = self.remote.as_ref().map(|r| r.activation).or(self.approach.as_ref().map(|(id, _)| *id));
        self.approach = None;
        match captured {
            // Already capturing: the visit in progress ends (letting go of
            // what it held) and broadcast takes over its capture.
            Some(activation) => {
                self.end_visit();
                self.begin_broadcast(activation, Some(reply));
            }
            None => {
                let side = self.peers[&edge].side;
                self.broadcast = Some(Broadcast::Starting { until: Instant::now() + SWITCH_TIMEOUT, edge, reply: Some(reply) });
                self.update_sides(); // arms `side` even while the pointer is locked
                let _ = self.capture.send(CaptureCmd::Activate { side });
            }
        }
    }

    fn no_targets(&self) -> String {
        if self.peers.is_empty() {
            "error no other computer is connected, so there is nothing to broadcast to; `one-mk status` shows which ones are online\n".into()
        } else {
            "error the connected computers have not said what their screens are yet; try again in a moment\n".into()
        }
    }

    /// Capture `id` started. True if it was for broadcast (it is now on,
    /// or handed straight back).
    pub(super) fn broadcast_activated(&mut self, id: u32) -> bool {
        match self.broadcast.take() {
            Some(Broadcast::Starting { reply, .. }) => {
                self.begin_broadcast(id, reply);
                true
            }
            Some(Broadcast::Cancelled { .. }) => {
                tracing::debug!("capture started after broadcast was stopped; handing it back");
                self.release_capture(id, None);
                self.update_sides();
                true
            }
            Some(Broadcast::On(mut on)) => {
                // A new capture superseded ours: broadcast carries on with it.
                on.activation = id;
                self.broadcast = Some(Broadcast::On(on));
                true
            }
            None => false,
        }
    }

    /// Enters every target on capture `activation`.
    fn begin_broadcast(&mut self, activation: u32, reply: Option<oneshot::Sender<String>>) {
        let targets = self.broadcast_targets();
        let Some(&edge) = targets.first() else {
            // Every computer left while capture was starting.
            self.release_capture(activation, None);
            self.update_sides();
            return answer(reply, self.no_targets());
        };
        let mut visits = VecDeque::with_capacity(targets.len());
        for fp in targets {
            visits.extend(self.broadcast_visit(activation, fp, None));
        }
        self.broadcast = Some(Broadcast::On(On {
            activation,
            edge,
            visits,
            resting: Vec::new(),
            last_input: Instant::now(),
            actions: Vec::new(),
            deltas: Vec::with_capacity(MAX_COALESCE),
        }));
        self.update_sides();
        let names = self.broadcast_names();
        let (title, body) = started_text(&names, &self.broadcast_hotkey());
        println!("{title}. {body}");
        crate::notify::info(title, body);
        answer(reply, reply_text("ok", &names));
    }

    /// A broadcast visit to `fp` (its `Enter` sent), its pointer at `at` or
    /// the centre of its main screen. None if it has no screens.
    fn broadcast_visit(&mut self, activation: u32, fp: Fingerprint, at: Option<Pos>) -> Option<Remote> {
        let p = self.peers.get(&fp)?;
        let (desktop, side) = (p.desktop.clone()?, p.side);
        self.epoch = self.epoch.wrapping_add(1);
        let r = self.visit(activation, fp, cursor_for(desktop, side, at, self.epoch));
        tracing::debug!(peer = %self.name_for(fp), "broadcast: entered");
        Some(r)
    }

    /// Ends broadcast: every computer lets go of what it holds, and the
    /// pointer comes back to the middle of this computer's main screen.
    /// False if it was not on.
    pub(super) fn stop_broadcast(&mut self, end: End) -> bool {
        let n = self.broadcast_names().len();
        let on = match self.broadcast.take() {
            None => return false,
            Some(Broadcast::Cancelled { until }) => {
                self.broadcast = Some(Broadcast::Cancelled { until });
                return false;
            }
            Some(Broadcast::Starting { reply, .. }) => {
                answer(reply, "error broadcast was stopped before it began\n".into());
                self.broadcast = Some(Broadcast::Cancelled { until: Instant::now() + LATE_CAPTURE });
                self.update_sides();
                return true;
            }
            Some(Broadcast::On(on)) => on,
        };
        for r in on.visits {
            self.remote = Some(r);
            self.end_visit();
        }
        let center = self.local.as_ref().map(|d| d.primary().center());
        self.release_capture(on.activation, center);
        self.update_sides();
        let why = end.why(self.broadcast_idle_off());
        tracing::info!(computers = n, "broadcast stopped: {why}");
        println!("Broadcast stopped: {why}.");
        if end != End::Quit {
            crate::notify::info("Broadcast stopped".into(), format!("{}. The mouse and keyboard are back on this computer.", capitalized(&why)));
        }
        true
    }

    // ------------------------------------------------------------- input

    /// Local input broadcast handles: input and the end of its capture.
    pub(super) fn broadcast_takes(&self, ev: &Event) -> bool {
        let Some(on) = self.on_ref() else { return false };
        match ev {
            Event::Motion { .. }
            | Event::Key { .. }
            | Event::Button { .. }
            | Event::Scroll { .. }
            | Event::ScrollDiscrete { .. }
            | Event::ScrollStop => true,
            Event::Deactivated { id } => *id == on.activation,
            _ => false,
        }
    }

    pub(super) fn broadcast_input(&mut self, ev: Event, events: &mut mpsc::UnboundedReceiver<Event>) {
        if let Event::Deactivated { .. } = ev {
            tracing::info!("capture ended by the system; broadcast stops");
            self.stop_broadcast(End::CaptureLost);
            return;
        }
        if let Event::Key { code, down, .. } = ev {
            let m = self.mods;
            if down && code == KEY_ESC && [Modifier::Ctrl, Modifier::Alt, Modifier::Shift].into_iter().all(|k| m.has(k)) {
                tracing::info!("emergency shortcut: broadcast stops, pointer is home");
                self.stop_broadcast(End::Emergency);
                return;
            }
            self.mods.update(code, down);
        }
        self.rejoin_resting();
        if let Some(on) = self.on_mut() {
            on.last_input = Instant::now();
        }
        match ev {
            Event::Motion { dx, dy, device } => {
                if let Some(next) = self.broadcast_motion(dx, dy, device, events) {
                    self.on_input(next, events);
                }
            }
            // Remapped (then Ctrl/Cmd swapped) per computer in `tune.rs`,
            // with the rules of the device it came from.
            Event::Key { code, down, device } | Event::Button { button: code, down, device } => {
                self.each_visit(|app| app.tuned_key(code, down, device));
            }
            Event::Scroll { dx, dy, device } => self.each_visit(|app| app.tuned_scroll(ScrollIn::Smooth(dx, dy), device)),
            Event::ScrollDiscrete { dx, dy, device } => {
                self.each_visit(|app| app.tuned_scroll(ScrollIn::Discrete(dx, dy), device));
            }
            Event::ScrollStop => self.each_visit(|app| app.tuned_scroll(ScrollIn::Stop, None)),
            _ => {}
        }
    }

    /// Moves every computer's pointer by a run of motion (what queued up
    /// meanwhile included). Returns the event that ended the run.
    fn broadcast_motion(
        &mut self,
        dx: f64,
        dy: f64,
        device: Option<DeviceRef>,
        events: &mut mpsc::UnboundedReceiver<Event>,
    ) -> Option<Event> {
        let Some(Broadcast::On(on)) = self.broadcast.as_mut() else { return None };
        on.deltas.clear();
        on.deltas.push((dx, dy, device));
        let mut next = None;
        while on.deltas.len() < MAX_COALESCE {
            match events.try_recv() {
                Ok(Event::Motion { dx, dy, device }) => on.deltas.push((dx, dy, device)),
                Ok(ev) => {
                    next = Some(ev);
                    break;
                }
                Err(_) => break,
            }
        }
        for r in &mut on.visits {
            let tune = &r.tune;
            if let Some(d) = advance(&mut r.cursor, |dx, dy, device| tune.motion(dx, dy, device), &on.deltas)
                && r.acked
            {
                r.send_motion(&self.peers, d);
            }
        }
        next
    }

    /// Runs `f` once per visit, with the visit in `self.remote` (where the
    /// single visit's code looks for it), then the key mapper actions they
    /// asked for, once each. Allocates nothing.
    pub(super) fn each_visit(&mut self, mut f: impl FnMut(&mut App)) {
        let Some(on) = self.on_mut() else { return };
        let mut visits = std::mem::take(&mut on.visits);
        debug_assert!(self.remote.is_none(), "a visit and broadcast at once");
        for _ in 0..visits.len() {
            let Some(r) = visits.pop_front() else { break };
            self.remote = Some(r);
            f(self);
            visits.extend(self.remote.take());
        }
        let Some(on) = self.on_mut() else { return };
        on.visits = visits;
        let mut actions = std::mem::take(&mut on.actions);
        for (i, a) in actions.iter().enumerate() {
            if !actions[..i].contains(a) {
                self.run_action(a);
            }
        }
        actions.clear();
        if let Some(on) = self.on_mut() {
            on.actions = actions; // keeps the capacity
        }
    }

    /// A key mapper action asked for while broadcasting: held to run once,
    /// after every computer had the event (a scroll goes to each). False
    /// when not broadcasting: run it now.
    pub(super) fn defer_action(&mut self, a: &Action) -> bool {
        let Some(on) = self.on_mut() else { return false };
        if matches!(a, Action::Scroll { .. }) {
            return false;
        }
        on.actions.push(a.clone());
        true
    }

    /// Computers that let go by their idle rule are entered again (their
    /// input waits until they accept, as for any visit).
    fn rejoin_resting(&mut self) {
        let Some(on) = self.on_mut() else { return };
        if on.resting.is_empty() {
            return;
        }
        let (activation, resting) = (on.activation, std::mem::take(&mut on.resting));
        for (fp, at) in resting {
            let Some(r) = self.broadcast_visit(activation, fp, Some(at)) else { continue };
            tracing::info!(peer = %self.name_for(fp), "broadcast: entered again after its idle rule let go");
            if let Some(on) = self.on_mut() {
                on.visits.push_back(r);
            }
        }
    }

    // ------------------------------------------------------ the computers

    /// A control message from `fp`. Those about its broadcast visit are
    /// handled here (None); the rest go on.
    pub(super) fn broadcast_control(&mut self, fp: Fingerprint, msg: Control) -> Option<Control> {
        let now = Instant::now();
        let Some(Broadcast::On(on)) = self.broadcast.as_mut() else { return Some(msg) };
        let Some(r) = on.visits.iter_mut().find(|r| r.peer == fp) else { return Some(msg) };
        let epoch = r.cursor.epoch();
        let out = match msg {
            Control::EnterAck { epoch: e } if e == epoch => {
                // A late answer is ignored: the timer leaves it out.
                if !r.acked && now < r.entered_at + ENTER_TIMEOUT {
                    r.accepted(&self.peers, now);
                }
                return None;
            }
            Control::Busy { epoch: e } if e == epoch => Some((Out::Leave, "it is controlled by another computer")),
            // Only the idle rule's own message rests; a plain `Revoked` is the
            // user taking control back (or an older one-mk, which cannot say):
            // it leaves until the broadcast is started again.
            Control::IdleRevoked { epoch: e } if e == epoch => {
                Some((Out::Rest, "it let go after a minute without input; your next input enters it again"))
            }
            Control::Revoked { epoch: e } if e == epoch => Some((Out::Leave, "it took back control")),
            // Broadcast never asks what has the focus there.
            Control::Focus { epoch: e, .. } if e == epoch => return None,
            Control::Screens(ref screens) => {
                if screens.is_empty() {
                    Some((Out::Leave, "it has no screens"))
                } else {
                    let moved = r.cursor.set_desktop(Desktop::new(screens.clone()));
                    if r.acked {
                        r.send_motion(&self.peers, moved);
                    }
                    None
                }
            }
            _ => None,
        };
        match out {
            Some((how, why)) => {
                self.broadcast_drop(fp, how, why);
                // Its monitors are still recorded as usual.
                matches!(msg, Control::Screens(_)).then_some(msg)
            }
            None => Some(msg),
        }
    }

    /// Peer `fp` disconnected, or its link was replaced.
    pub(super) fn broadcast_peer_gone(&mut self, fp: Fingerprint) {
        self.broadcast_drop(fp, Out::Gone, "it disconnected");
    }

    /// Takes `fp` out of the broadcast; it ends when no computer is left.
    fn broadcast_drop(&mut self, fp: Fingerprint, how: Out, why: &str) {
        let name = self.name_for(fp);
        let Some(on) = self.on_mut() else { return };
        let rested = on.resting.iter().position(|(f, _)| *f == fp).map(|i| on.resting.swap_remove(i));
        let visit = on.visits.iter().position(|r| r.peer == fp).and_then(|i| on.visits.remove(i));
        if rested.is_none() && visit.is_none() {
            return;
        }
        match (how, visit) {
            (Out::Rest, Some(r)) => {
                tracing::info!(peer = %name, "broadcast: {why}");
                if let Some(on) = self.on_mut() {
                    on.resting.push((fp, r.cursor.pos()));
                }
                // It released everything itself; this record of it goes.
                return;
            }
            (Out::Leave, Some(r)) => {
                self.remote = Some(r);
                self.end_visit();
            }
            _ => {}
        }
        if how != Out::Rest {
            println!("{name} left the broadcast: {why}.");
        }
        if self.on_ref().is_some_and(|on| on.visits.is_empty() && on.resting.is_empty()) {
            self.stop_broadcast(End::NoneLeft);
        }
    }

    // ------------------------------------------------------------ timers

    pub(super) fn broadcast_deadline(&self) -> Option<Instant> {
        match &self.broadcast {
            None => None,
            Some(Broadcast::Starting { until, .. } | Broadcast::Cancelled { until }) => Some(*until),
            Some(Broadcast::On(on)) => {
                let visits = on.visits.iter().map(|r| {
                    let tap = r.tune.deadline().map(Instant::from_std);
                    tap.map_or(r.deadline(), |t| t.min(r.deadline()))
                });
                let idle = self.broadcast_idle_off().map(|d| on.last_input + d);
                visits.chain(idle).min()
            }
        }
    }

    pub(super) fn broadcast_timer(&mut self, now: Instant) {
        match &self.broadcast {
            Some(Broadcast::Starting { until, .. }) if now >= *until => {
                tracing::warn!("could not start broadcast: this computer's input capture did not start");
                if let Some(Broadcast::Starting { reply, .. }) = self.broadcast.take() {
                    answer(reply, "error this computer's input capture did not start, so broadcast could not begin; `one-mk doctor` checks why\n".into());
                }
                self.broadcast = Some(Broadcast::Cancelled { until: now + LATE_CAPTURE });
                self.update_sides();
                return;
            }
            Some(Broadcast::Cancelled { until }) if now >= *until => {
                self.broadcast = None;
                return;
            }
            Some(Broadcast::On(_)) => {}
            _ => return,
        }
        if let (Some(on), Some(idle)) = (self.on_ref(), self.broadcast_idle_off())
            && now >= on.last_input + idle
        {
            self.stop_broadcast(End::Idle);
            return;
        }
        self.each_visit(|app| app.tune_timer(now.into_std()));
        let mut late = Vec::new();
        if let Some(Broadcast::On(on)) = self.broadcast.as_mut() {
            for r in &mut on.visits {
                if r.acked {
                    r.keepalive(&self.peers, now);
                } else if now >= r.entered_at + ENTER_TIMEOUT {
                    late.push(r.peer);
                }
            }
        }
        for fp in late {
            self.broadcast_drop(fp, Out::Leave, "it did not accept the pointer in time");
        }
    }

    // ------------------------------------------------------------- the rest

    /// Asks each computer in the broadcast to keep its screen awake, as a
    /// visit does (`[power] block_screensaver_on_peers`).
    pub(super) fn broadcast_power(&mut self, block: bool) {
        let Some(Broadcast::On(on)) = self.broadcast.as_mut() else { return };
        for r in &mut on.visits {
            let Some(p) = self.peers.get(&r.peer) else { continue };
            if let Some(want) = crate::power::inhibit_request(block, r.acked, p.caps, r.inhibit_sent) {
                r.inhibit_sent = want;
                p.ctl.send(Control::Inhibit { epoch: r.cursor.epoch(), on: want });
            }
        }
    }

    /// `settings.toml` was reloaded: each computer follows the rules that
    /// apply to it now (see `App::retune`).
    pub(super) fn broadcast_retune(&mut self) {
        self.each_visit(App::retune);
    }
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// Shorter times for the daemon tests, per settings folder.
#[cfg(test)]
pub(super) mod testing {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::Duration;

    type Times = (Option<Option<Duration>>, Option<Duration>);
    static TIMES: Mutex<Option<HashMap<PathBuf, Times>>> = Mutex::new(None);

    /// The idle auto-off (`Some(None)`: never) and the idle let-go
    /// threshold for the daemon using `dir`.
    pub fn set(dir: &Path, idle_off: Option<Option<Duration>>, let_go: Option<Duration>) {
        TIMES.lock().unwrap().get_or_insert_default().insert(dir.to_owned(), (idle_off, let_go));
    }

    pub(super) fn get(dir: &Path) -> Times {
        TIMES.lock().unwrap().as_ref().and_then(|m| m.get(dir).copied()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_proto::{Rect, Side};

    fn pos(d: Option<Datagram>) -> Option<Pos> {
        match d? {
            Datagram::Motion { pos, .. } => Some(pos),
            other => panic!("expected a position, got {other:?}"),
        }
    }

    /// Every computer's pointer moves by the same deltas from its own
    /// centre and stops at its own screens' edges; there is no way home,
    /// and nothing is sent for a pointer pinned in a corner.
    #[test]
    fn each_pointer_moves_relatively_and_is_clamped_to_its_own_screens() {
        let big = Desktop::new(vec![Rect::new(0, 0, 2560, 1440)]);
        let small = Desktop::new(vec![Rect::new(0, 0, 1280, 800)]);
        let two = Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1920, 1080)]);
        let mut a = cursor_for(big, Side::Right, None, 1);
        let mut b = cursor_for(small, Side::Left, None, 2);
        let mut c = cursor_for(two, Side::Top, None, 3);
        assert_eq!((a.pos(), b.pos(), c.pos()), (Pos::new(1280.0, 720.0), Pos::new(640.0, 400.0), Pos::new(960.0, 540.0)));
        let same = |dx, dy, _| (dx, dy);
        let run = [(100.0, 50.0, None), (-30.0, 10.0, None)];
        assert_eq!(pos(advance(&mut a, same, &run)), Some(Pos::new(1350.0, 780.0)));
        assert_eq!(pos(advance(&mut b, same, &run)), Some(Pos::new(710.0, 460.0)));
        // A huge push right: each stops at its own right edge, never leaves.
        let far = [(10_000.0, 0.0, None)];
        let at_edge = |p: Option<Pos>, right: f64| p.is_some_and(|p| (right - 1.0..right).contains(&p.x));
        assert!(at_edge(pos(advance(&mut a, same, &far)), 2560.0));
        assert!(at_edge(pos(advance(&mut b, same, &far)), 1280.0));
        assert!(at_edge(pos(advance(&mut c, same, &far)), 3840.0), "across both monitors");
        // Through the side facing home too (left for a computer on our right).
        assert_eq!(pos(advance(&mut a, same, &[(-99_999.0, -99_999.0, None)])), Some(Pos::new(0.0, 0.0)));
        assert_eq!(advance(&mut a, same, &[(-5.0, -5.0, None)]), None, "pinned in the corner: nothing to send");
        // Applied in order, clamping in between (not summed): 0 -> 0 -> 100.
        assert_eq!(pos(advance(&mut a, same, &[(-50.0, 0.0, None), (100.0, 0.0, None)])).map(|p| p.x), Some(100.0));
        // Each computer's own pointer speed applies.
        let mut d = cursor_for(Desktop::new(vec![Rect::new(0, 0, 1000, 1000)]), Side::Right, None, 4);
        assert_eq!(pos(advance(&mut d, |dx, dy, _| (dx * 2.0, dy * 2.0), &[(10.0, 10.0, None)])), Some(Pos::new(520.0, 520.0)));
        // Motion keeps its epoch, and every move a new seq.
        let (Some(Datagram::Motion { epoch, seq: s1, .. }), Some(Datagram::Motion { seq: s2, .. })) =
            (advance(&mut d, same, &[(1.0, 0.0, None)]), advance(&mut d, same, &[(1.0, 0.0, None)]))
        else {
            panic!()
        };
        assert_eq!(epoch, 4);
        assert!(s2 > s1);
    }

    /// An odd monitor arrangement: a pointer starting on the primary of
    /// two monitors of different heights stays inside real screen area.
    #[test]
    fn a_pointer_never_ends_up_between_monitors() {
        let odd = Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1280, 720)]);
        let mut c = cursor_for(odd.clone(), Side::Right, None, 1);
        for run in [[(1500.0, 0.0, None)], [(0.0, 400.0, None)], [(-5000.0, 5000.0, None)], [(9000.0, -9000.0, None)]] {
            if let Some(p) = pos(advance(&mut c, |dx, dy, _| (dx, dy), &run)) {
                assert!(odd.screens.iter().any(|s| s.contains(p)), "{p:?} is off every screen");
            }
        }
    }

    #[test]
    fn words() {
        let n = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(joined(&n(&["a"])), "a");
        assert_eq!(joined(&n(&["a", "b"])), "a and b");
        assert_eq!(joined(&n(&["a", "b", "c"])), "a, b and c");
        assert_eq!(
            started_text(&n(&["desk", "mac"]), "Ctrl+Alt+B"),
            ("Broadcasting to 2 computers".into(), "Everything you type, click and scroll goes to desk and mac. Ctrl+Alt+B stops it.".into())
        );
        assert_eq!(started_text(&n(&["desk"]), "").0, "Broadcasting to 1 computer");
        assert!(started_text(&n(&["desk"]), "").1.ends_with("`one-mk broadcast off` stops it."));
        assert_eq!(reply_text("ok", &n(&["a", "b"])), "ok\non\na\tb\n");
        assert_eq!(reply_text("already", &[]), "already\noff\n");
        assert_eq!(End::Idle.why(Some(Duration::from_secs(15 * 60))), "no input for 15 minutes");
        assert_eq!(End::Idle.why(Some(Duration::from_secs(60))), "no input for 1 minute");
        assert_eq!(capitalized(&End::Emergency.why(None)), "Ctrl+Alt+Shift+Esc");
        assert_eq!(capitalized(&End::Asked.why(None)), "You stopped it");
        assert_eq!(capitalized(""), "");
    }
}
