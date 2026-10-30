//! Input tuning in the send path of a visit: the key mapper, pointer
//! speed, scroll tuning and button remapping (`[input]`, see
//! `crate::tuning` and `docs/INPUT-TUNING.md`). Hotkeys have seen the
//! physical keys before anything here; the Ctrl/Cmd swap applies after.
//! Input from a keyboard or mouse with rules of its own (per-device rules)
//! uses those on top of the rest; see "Per-device rules" in the doc.

use super::App;
use crate::control::Request;
use crate::inputdevices::{DaemonNote, Remembered};
use crate::tuning::{PeerFacts, ResolvedSet};
use one_mk_ctl::client::{SwitchTo, Toggle};
use one_mk_input::{DeviceRef, InputDevice};
use one_mk_net::Fingerprint;
use one_mk_proto::keynames::is_button;
use one_mk_proto::pointer::PointerTune;
use one_mk_proto::remap::{Action, Emit, KeymapSet, Mapper};
use one_mk_proto::scroll::{ScrollOut, ScrollTune};
use one_mk_proto::{Control, swap_ctrl_meta};
use std::path::{Path, PathBuf};
use tokio::sync::oneshot;

/// The tuning of one visit, resolved for its computer.
pub(super) struct Tune {
    mapper: Mapper,
    pointer: PointerTune,
    scroll: ScrollTune,
    /// Devices with rules of their own: their pointer tuning and their
    /// own scrolling (remainders of one wheel never carry into another's).
    by_device: Vec<(DeviceRef, PointerTune, ScrollTune)>,
    /// The device whose scrolling came last: a `ScrollStop` is for it.
    last_scroll: Option<DeviceRef>,
    /// What the mapper emitted and is still to be sent; kept between
    /// events so its capacity is reused.
    out: Vec<Emit>,
}

impl Tune {
    pub(super) fn new(r: ResolvedSet) -> Self {
        let (keymaps, by_device) = Self::split(&r);
        Tune {
            mapper: Mapper::with_set(keymaps),
            pointer: r.base.pointer,
            scroll: ScrollTune::new(r.base.scroll),
            by_device,
            last_scroll: None,
            out: Vec::new(),
        }
    }

    /// The keymaps, and the pointer and scroll tuning of each device with
    /// rules of its own.
    fn split(r: &ResolvedSet) -> (KeymapSet, Vec<(DeviceRef, PointerTune, ScrollTune)>) {
        let mut keymaps = KeymapSet::new(r.base.keymap.clone());
        let mut by_device = Vec::new();
        for (ids, d) in &r.devices {
            keymaps.add(ids, d.keymap.clone());
            by_device.extend(ids.iter().map(|&id| (id, d.pointer, ScrollTune::new(d.scroll))));
        }
        (keymaps, by_device)
    }

    /// When the mapper's timer must fire: only while a tap/hold key is undecided.
    pub(super) fn deadline(&self) -> Option<std::time::Instant> {
        self.mapper.deadline()
    }

    /// One motion report from `device`, before the remote cursor adds it up.
    pub(super) fn motion(&self, dx: f64, dy: f64, device: Option<DeviceRef>) -> (f64, f64) {
        let pointer = device.and_then(|d| self.by_device.iter().find(|x| x.0 == d)).map_or(&self.pointer, |x| &x.1);
        pointer.apply(dx, dy, crate::tuning::RAW_MOTION)
    }

    /// The scrolling of `device` (`None`: not known).
    fn scroll(&mut self, device: Option<DeviceRef>) -> &mut ScrollTune {
        match device.and_then(|d| self.by_device.iter().position(|x| x.0 == d)) {
            Some(i) => &mut self.by_device[i].2,
            None => &mut self.scroll,
        }
    }

    /// Takes the rules that apply now: what is held stays held (see
    /// `Mapper::set_keymaps`), scrolling keeps its remainders where its
    /// rules stay the same.
    fn replace(&mut self, new: Tune, out: &mut Vec<Emit>) {
        self.pointer = new.pointer;
        if self.scroll.config() != new.scroll.config() {
            self.scroll = new.scroll;
        }
        let mut by_device = new.by_device;
        for (id, _, scroll) in &mut by_device {
            if let Some(old) = self.by_device.iter_mut().find(|x| x.0 == *id && x.2.config() == scroll.config()) {
                std::mem::swap(scroll, &mut old.2);
            }
        }
        self.by_device = by_device;
        self.mapper.set_keymaps(new.mapper.keymaps().clone(), &mut |e| out.push(e));
    }
}

/// The keyboards and mice connected here, as the backend said, and the
/// ones seen before (`input-devices.toml`).
pub(super) struct Devices {
    dir: PathBuf,
    /// Connected now (`None`: the backend hasn't said).
    connected: Option<Vec<InputDevice>>,
    /// Why the backend can't tell devices apart, if it said so.
    unavailable: Option<&'static str>,
    memory: Remembered,
}

impl Devices {
    /// The devices seen before; none counts as connected until the backend
    /// says (a daemon that was killed couldn't say what went away since).
    pub(super) fn load(dir: &Path, backend: &str) -> Self {
        let mut memory = Remembered::load(dir).unwrap_or_else(|e| {
            tracing::warn!("{e:#}; starting a new list of input devices");
            Remembered::default()
        });
        memory.daemon = DaemonNote { backend: backend.to_owned(), unavailable: None };
        memory.disconnect_all(crate::inputdevices::now());
        let d = Devices { dir: dir.to_owned(), connected: None, unavailable: None, memory };
        d.save();
        d
    }

    pub(super) fn list(&self) -> &[InputDevice] {
        self.connected.as_deref().unwrap_or_default()
    }

    fn save(&self) {
        if let Err(e) = self.memory.save(&self.dir) {
            tracing::warn!("can't write {}: {e:#}", crate::inputdevices::path(&self.dir).display());
        }
    }

    /// Writes down what is connected now, if that changed.
    fn remember(&mut self) {
        let unavailable = self.unavailable.map(str::to_owned);
        let note_changed = self.memory.daemon.unavailable != unavailable;
        self.memory.daemon.unavailable = unavailable;
        let list = self.connected.clone().unwrap_or_default();
        if self.memory.update(&list, crate::inputdevices::now()) || note_changed {
            self.save();
        }
    }

    /// The daemon stops: nothing is known to be connected any more.
    pub(super) fn stopped(&mut self) {
        if self.memory.disconnect_all(crate::inputdevices::now()) {
            self.save();
        }
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
        let facts = PeerFacts { name, id: &id, os, apps, device: None };
        let r = self.settings.input.resolve_devices(&facts, swap, self.devices.list());
        for w in r.base.warnings.iter().chain(r.devices.iter().flat_map(|(_, d)| &d.warnings)) {
            tracing::debug!(peer = %name, "input tuning: {w}");
        }
        Tune::new(r)
    }

    /// The backend says which keyboards and mice are connected now. During
    /// a visit, whatever a device that went away (unplugged, asleep) held
    /// is let go of at once, and devices with rules of their own get them.
    pub(super) fn on_devices(&mut self, list: Vec<InputDevice>) {
        // A number reused for another device counts as one gone, too.
        let gone: Vec<DeviceRef> = self.devices.list().iter().filter(|d| !list.contains(d)).map(|d| d.id).collect();
        tracing::debug!(devices = list.len(), gone = gone.len(), "keyboards and mice connected");
        self.devices.connected = Some(list);
        self.devices.unavailable = None;
        if let Some(r) = self.remote.as_mut()
            && !gone.is_empty()
        {
            let t = &mut r.tune;
            let out = &mut t.out;
            for &d in &gone {
                t.mapper.device_gone(d, &mut |e| out.push(e));
            }
            let still: Vec<u32> = r.held.iter().filter(|&c| !t.mapper.held_anywhere(c)).collect();
            for code in still {
                r.held.update(code, false);
            }
            self.send_tuned();
        }
        self.retune();
        self.devices.remember();
    }

    /// The backend can't tell keyboards and mice apart (the desktop merges
    /// them): rules for one device never apply here.
    pub(super) fn on_devices_unavailable(&mut self, why: &'static str) {
        tracing::info!("{why}");
        self.devices.unavailable = Some(why);
        self.devices.connected = Some(Vec::new());
        self.devices.remember();
    }

    /// Why rules for one device are not in force, for `one-mk status`.
    fn device_rule_notes(&self) -> Option<String> {
        if !self.settings.input.uses_devices() {
            return None;
        }
        let support = one_mk_input::device_support(self.backend_name);
        match (self.devices.unavailable, &self.devices.connected, support) {
            (Some(why), ..) | (None, None, Err(why)) => Some(format!("Rules for one keyboard or mouse are not in force: {why}.")),
            (None, _, Ok(Some(untested))) => Some(format!("Rules for one keyboard or mouse: {untested}.")),
            // The backend lists devices: they work.
            (None, Some(_), _) | (None, None, Ok(None)) => None,
        }
    }

    /// The apps that rules for `fp` name: while there are any, the visit
    /// asks it which app has its focus.
    pub(super) fn app_rules_for(&self, fp: Fingerprint) -> Vec<String> {
        let Some(p) = self.peers.get(&fp) else { return Vec::new() };
        let id = fp.to_string();
        self.settings.input.apps_for(&PeerFacts { name: &p.name, id: &id, os: p.os, apps: &[], device: None })
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
        out.extend(self.device_rule_notes());
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
        let mut out = std::mem::take(&mut r.tune.out);
        r.tune.replace(new, &mut out);
        r.tune.out = out;
        self.send_tuned();
    }

    /// A physical key or button transition during a visit, from `device`
    /// (`None`: not known). A release of something not pressed during the
    /// visit does nothing (the mapper knows what was).
    pub(super) fn tuned_key(&mut self, code: u32, down: bool, device: Option<DeviceRef>) {
        let Some(r) = self.remote.as_mut() else { return };
        let now = tokio::time::Instant::now().into_std();
        let t = &mut r.tune;
        let out = &mut t.out;
        t.mapper.key_from(code, down, device, now, &mut |e| out.push(e));
        // Held here from any device: what shortcuts and focus watching see.
        if down || !t.mapper.held_anywhere(code) {
            r.held.update(code, down);
        }
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
    pub(super) fn tuned_scroll(&mut self, input: ScrollIn, device: Option<DeviceRef>) {
        let Some(r) = self.remote.as_mut() else { return };
        let t = &mut r.tune;
        let mut out = std::mem::take(&mut t.out);
        let mods = t.mapper.mods();
        let tuned = match input {
            ScrollIn::Stop => {
                let last = t.last_scroll;
                one_mk_proto::scroll::Tuned { out: t.scroll(last).stop(), used: 0 }
            }
            ScrollIn::Smooth(dx, dy) => {
                t.mapper.interrupt(&mut |e| out.push(e));
                t.last_scroll = device;
                t.scroll(device).smooth(dx, dy, mods)
            }
            ScrollIn::Discrete(dx, dy) => {
                t.mapper.interrupt(&mut |e| out.push(e));
                t.last_scroll = device;
                t.scroll(device).discrete(dx, dy, mods)
            }
        };
        if tuned.used != 0 {
            t.mapper.lift(tuned.used, &mut |e| out.push(e));
        }
        t.out = out;
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
            self.run_action(&a);
        }
    }

    /// A key mapped to an action: the same requests `one-mk switch`,
    /// `lock-pointer` and `lock-all` send, scrolling, or a command run
    /// like a hook.
    fn run_action(&mut self, action: &Action) {
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
        let base = s.resolve(&PeerFacts { name: "b", id: "", os: os::LINUX, apps: &[], device: None }, false);
        Tune::new(ResolvedSet { base, devices: Vec::new() })
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
