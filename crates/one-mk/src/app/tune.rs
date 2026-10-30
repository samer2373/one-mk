//! Input tuning in the send path of a visit: the key mapper, pointer
//! speed, scroll tuning and button remapping (`[input]`, see
//! `crate::tuning` and `docs/INPUT-TUNING.md`). Hotkeys have seen the
//! physical keys before anything here; the Ctrl/Cmd swap applies after.

use super::App;
use crate::control::Request;
use crate::tuning::{PeerFacts, Resolved};
use one_mk_ctl::client::{SwitchTo, Toggle};
use one_mk_net::Fingerprint;
use one_mk_proto::keynames::is_button;
use one_mk_proto::pointer::PointerTune;
use one_mk_proto::remap::{Action, Emit, Mapper};
use one_mk_proto::scroll::{ScrollOut, ScrollTune};
use one_mk_proto::{Control, swap_ctrl_meta};
use tokio::sync::oneshot;

/// The tuning of one visit, resolved for its computer.
pub(super) struct Tune {
    mapper: Mapper,
    pointer: PointerTune,
    scroll: ScrollTune,
    /// What the mapper emitted and is still to be sent; kept between
    /// events so its capacity is reused.
    out: Vec<Emit>,
}

impl Tune {
    pub(super) fn new(r: Resolved) -> Self {
        Tune { mapper: Mapper::new(r.keymap), pointer: r.pointer, scroll: ScrollTune::new(r.scroll), out: Vec::new() }
    }

    /// When the mapper's timer must fire: only while a tap/hold key is undecided.
    pub(super) fn deadline(&self) -> Option<std::time::Instant> {
        self.mapper.deadline()
    }

    /// One motion report, before the remote cursor adds it up.
    pub(super) fn motion(&self, dx: f64, dy: f64) -> (f64, f64) {
        self.pointer.apply(dx, dy, crate::tuning::RAW_MOTION)
    }
}

/// A scroll event from the backend.
pub(super) enum ScrollIn {
    Smooth(f64, f64),
    Discrete(i32, i32),
    Stop,
}

impl App {
    /// The tuning for a visit to `fp`, whose Ctrl/Cmd are swapped or not,
    /// while the app going by `apps` has the focus there (none: unknown).
    pub(super) fn tune_for(&self, fp: Fingerprint, swap: bool, apps: &[String]) -> Tune {
        let (name, os) = self.peers.get(&fp).map_or(("", one_mk_proto::os::UNKNOWN), |p| (p.name.as_str(), p.os));
        let id = fp.to_string();
        let r = self.settings.input.resolve(&PeerFacts { name, id: &id, os, apps }, swap);
        for w in &r.warnings {
            tracing::debug!(peer = %name, "input tuning: {w}");
        }
        Tune::new(r)
    }

    /// The apps that rules for `fp` name: while there are any, the visit
    /// asks it which app has its focus.
    pub(super) fn app_rules_for(&self, fp: Fingerprint) -> Vec<String> {
        let Some(p) = self.peers.get(&fp) else { return Vec::new() };
        let id = fp.to_string();
        self.settings.input.apps_for(&PeerFacts { name: &p.name, id: &id, os: p.os, apps: &[] })
    }

    /// Why rules for apps on a connected computer are not in force, one
    /// line per computer, for `one-mk status`: it can't say which app has
    /// its focus (an older one-mk), or said it can't tell.
    pub(super) fn app_rule_notes(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (fp, p) in &self.peers {
            let apps = self.app_rules_for(*fp);
            let visit = self.remote.as_ref().filter(|r| r.peer == *fp);
            let why = if p.caps & one_mk_proto::caps::FOCUS_APP == 0 {
                "its one-mk is older than per-app rules"
            } else if visit.is_some_and(|r| r.apps_heard && r.peer_apps.is_empty()) {
                "it can't tell which app has the focus right now, as with Wayland windows on Linux"
            } else {
                continue;
            };
            if !apps.is_empty() {
                let rules = if apps.len() == 1 { "Rule for app" } else { "Rules for apps" };
                out.push(format!("{rules} {} not active: {} does not report its focused app ({why}).", apps.join(", "), p.name));
            }
        }
        out.sort();
        out
    }

    /// `settings.toml` was reloaded, or another app has the focus on the
    /// computer in use: the visit in progress follows the rules that apply
    /// now. What is held stays held, by the code it was pressed as, until
    /// released; a tap/hold key not decided yet counts as held and layers
    /// end (see `Mapper::set_keymap`); the same rules change nothing.
    pub(super) fn retune(&mut self) {
        let Some(r) = self.remote.as_ref() else { return };
        let new = self.tune_for(r.peer, r.swap, &r.peer_apps);
        let Some(r) = self.remote.as_mut() else { return };
        let t = &mut r.tune;
        t.pointer = new.pointer;
        if t.scroll.config() != new.scroll.config() {
            t.scroll = new.scroll;
        }
        let keymap = new.mapper.keymap().clone();
        let out = &mut t.out;
        t.mapper.set_keymap(keymap, &mut |e| out.push(e));
        self.send_tuned();
    }

    /// A physical key or button transition during a visit (already
    /// filtered: a release always follows a press).
    pub(super) fn tuned_key(&mut self, code: u32, down: bool) {
        let Some(r) = self.remote.as_mut() else { return };
        let now = tokio::time::Instant::now().into_std();
        let t = &mut r.tune;
        let out = &mut t.out;
        t.mapper.key(code, down, now, &mut |e| out.push(e));
        self.send_tuned();
    }

    /// The mapper's timer (a tap/hold key held long enough).
    pub(super) fn tune_timer(&mut self, now: std::time::Instant) {
        let Some(r) = self.remote.as_mut() else { return };
        if r.tune.deadline().is_none_or(|d| now < d) {
            return;
        }
        let t = &mut r.tune;
        let out = &mut t.out;
        t.mapper.timer(now, &mut |e| out.push(e));
        self.send_tuned();
    }

    /// Lets go of everything the mapper sent (the visit ends).
    pub(super) fn release_tuned(&mut self) {
        let Some(r) = self.remote.as_mut() else { return };
        let t = &mut r.tune;
        let out = &mut t.out;
        t.mapper.release_all(&mut |e| out.push(e));
        self.send_tuned();
    }

    /// Scrolling during a visit: a pending tap/hold key counts as held,
    /// modifiers that change scrolling are lifted off the target, then the
    /// tuned scroll goes out.
    pub(super) fn tuned_scroll(&mut self, input: ScrollIn) {
        let Some(r) = self.remote.as_mut() else { return };
        let t = &mut r.tune;
        let out = &mut t.out;
        let tuned = match input {
            ScrollIn::Stop => one_mk_proto::scroll::Tuned { out: t.scroll.stop(), used: 0 },
            ScrollIn::Smooth(dx, dy) => {
                t.mapper.interrupt(&mut |e| out.push(e));
                t.scroll.smooth(dx, dy, t.mapper.mods())
            }
            ScrollIn::Discrete(dx, dy) => {
                t.mapper.interrupt(&mut |e| out.push(e));
                t.scroll.discrete(dx, dy, t.mapper.mods())
            }
        };
        if tuned.used != 0 {
            t.mapper.lift(tuned.used, &mut |e| out.push(e));
        }
        self.send_tuned();
        match tuned.out {
            Some(ScrollOut::Smooth { dx, dy }) => self.send_input(Control::Scroll { dx: dx as f32, dy: dy as f32 }),
            Some(ScrollOut::Discrete { dx, dy }) => self.send_input(Control::ScrollDiscrete { dx, dy }),
            Some(ScrollOut::Stop) => self.send_input(Control::ScrollStop),
            None => {}
        }
    }

    /// Sends what the mapper emitted: keys (swapped if this visit swaps
    /// Ctrl and Cmd) and buttons (with the pointer's position), then runs
    /// its actions, which may end the visit.
    fn send_tuned(&mut self) {
        let Some(r) = self.remote.as_mut() else { return };
        let mut out = std::mem::take(&mut r.tune.out);
        let mut actions = Vec::new();
        for e in out.drain(..) {
            let Some(r) = self.remote.as_ref() else { break };
            let msg = match e {
                Emit::Key { code, down } if is_button(code) => {
                    Control::Button { button: code, down, seq: r.cursor.seq(), pos: r.cursor.pos() }
                }
                Emit::Key { code, down } => Control::Key { code: if r.swap { swap_ctrl_meta(code) } else { code }, down },
                Emit::Action(a) => {
                    actions.push(a);
                    continue;
                }
            };
            self.send_input(msg);
        }
        if let Some(r) = self.remote.as_mut() {
            r.tune.out = out; // keeps the capacity
        }
        for a in actions {
            // While broadcasting, each computer's mapper may ask: done
            // once, after all of them (see `broadcast.rs`).
            if !self.defer_action(&a) {
                self.run_action(&a);
            }
        }
    }

    /// A key mapped to an action: the same requests `one-mk switch`,
    /// `lock-pointer` and `lock-all` send, scrolling, or a command run
    /// like a hook.
    pub(super) fn run_action(&mut self, action: &Action) {
        let req = match action {
            Action::Switch(to) => match SwitchTo::parse(to) {
                Some(to) => Request::Switch(to),
                None => return tracing::warn!("key action {action}: say which computer"),
            },
            Action::LockPointer(t) => Request::LockPointer(match t {
                None => Toggle::Toggle,
                Some(true) => Toggle::On,
                Some(false) => Toggle::Off,
            }),
            Action::LockAll => Request::LockAll,
            Action::Scroll { dx, dy } => return self.send_input(Control::ScrollDiscrete { dx: *dx, dy: *dy }),
            Action::Run(cmd) => {
                let peer = self.remote.as_ref().and_then(|r| Some((self.peers.get(&r.peer)?.name.clone(), r.peer.short())));
                return self.hooks.run_action(&self.settings.hooks, cmd, peer);
            }
        };
        let (tx, mut rx) = oneshot::channel();
        self.on_request(req, tx);
        let reply = rx.try_recv().unwrap_or_default();
        match reply.strip_prefix("error ") {
            Some(e) => tracing::warn!("key action {action}: {}", e.trim()),
            None => tracing::info!(reply = reply.lines().next().unwrap_or(""), "key action {action}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::{KeyRule, TuningSettings};
    use one_mk_proto::os;
    use std::time::Duration;
    use tokio::time::Instant;

    fn tap_hold() -> Tune {
        let mut s = TuningSettings::default();
        s.keymap.rule =
            vec![KeyRule { from: "capslock".into(), tap: Some("esc".into()), hold: Some("lctrl".into()), ..Default::default() }];
        Tune::new(s.resolve(&PeerFacts { name: "b", id: "", os: os::LINUX, apps: &[] }, false))
    }

    /// The daemon's loop sleeps until `Tune::deadline` (as `next_deadline`
    /// does) and then calls the timer: with tokio's clock paused, the hold
    /// comes exactly at 200 ms, once, and there is no timer at all before
    /// the key is pressed, after it fired, or once a tap cancelled it.
    #[tokio::test(start_paused = true)]
    async fn the_tap_hold_timer_fires_once_and_only_while_waiting() {
        let mut t = tap_hold();
        let mut out = Vec::new();
        assert_eq!(t.deadline(), None, "idle: nothing to wake up for");
        let start = Instant::now();
        t.mapper.key(58, true, start.into_std(), &mut |e| out.push(e));
        assert!(out.is_empty());
        let deadline = t.deadline().expect("armed while undecided");
        tokio::time::sleep_until(Instant::from_std(deadline)).await;
        assert_eq!(start.elapsed(), Duration::from_millis(200));
        t.mapper.timer(Instant::now().into_std(), &mut |e| out.push(e));
        assert_eq!(out, [Emit::Key { code: 29, down: true }]);
        assert_eq!(t.deadline(), None, "fired once");
        t.mapper.key(58, false, Instant::now().into_std(), &mut |e| out.push(e));
        assert_eq!(out[1..], [Emit::Key { code: 29, down: false }]);

        // A tap within the time cancels it.
        out.clear();
        t.mapper.key(58, true, Instant::now().into_std(), &mut |e| out.push(e));
        tokio::time::sleep(Duration::from_millis(120)).await;
        t.mapper.timer(Instant::now().into_std(), &mut |e| out.push(e));
        assert!(out.is_empty(), "not yet");
        t.mapper.key(58, false, Instant::now().into_std(), &mut |e| out.push(e));
        assert_eq!(out, [Emit::Key { code: 1, down: true }, Emit::Key { code: 1, down: false }]);
        assert_eq!(t.deadline(), None, "cancelled");
    }
}
