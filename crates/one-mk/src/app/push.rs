//! Edges where no computer sits (`docs/EDGE-PUSH.md`): pointer wrap
//! (`[edge] wrap`) and the probes of layout learning (`[edge] learn`).
//!
//! Both arm the pieces of this desktop's outline that lead to no computer,
//! beside the edges that do, in the one `CaptureCmd::SetEdges` the backend
//! gets; the backends need nothing new. When one catches the pointer, the
//! capture is handed straight back: at the opposite edge for a wrap, where
//! it is for a probe (a wall that counts pushes). Input caught meanwhile
//! is typed here, as at any wall (`App::handed_back`). Nothing here runs
//! between captures: no timer, no task.

use super::App;
use crate::learn::{self, Act, Cooldown, Online, Pushes, Shown};
use crate::settings::Learn;
use one_mk_ctl::status::{Suggestion, device_arg, is_it_there, on_your};
use one_mk_net::{Device, Fingerprint, PlacedBy};
use one_mk_proto::{Edge, HOME_INSET, Pos, Side, Start, free_edges, sanitize_name, wrap_point};
use std::collections::HashMap;
use std::time::Instant;

/// What an armed edge where no computer sits does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// Brings the pointer in at the opposite edge.
    Wrap,
    /// Hands it straight back, and counts the push.
    Probe,
}

#[derive(Default)]
pub(super) struct EdgePush {
    /// The armed pieces of edge where no computer sits.
    edges: Vec<(Edge, Kind)>,
    /// Capture `.0` is held at a wrap edge by an edge rule (a dwell, a key
    /// to hold), and wraps if it crosses.
    held: Option<u32>,
    pushes: Pushes,
    cooldown: Cooldown<Fingerprint>,
    shown: Option<Shown<Fingerprint>>,
    /// When each peer last connected (the newest is the likeliest to be
    /// the one just put somewhere).
    connected: HashMap<Fingerprint, Instant>,
}

impl EdgePush {
    pub(super) fn is_armed(&self, edges: &[(Edge, Kind)]) -> bool {
        self.edges == edges
    }

    pub(super) fn arm(&mut self, edges: Vec<(Edge, Kind)>) {
        self.edges = edges;
    }

    pub(super) fn connected(&mut self, fp: Fingerprint) {
        self.connected.insert(fp, Instant::now());
    }

    /// A new capture began: an earlier hold at a wrap edge is over.
    pub(super) fn new_capture(&mut self) {
        self.held = None;
    }

    /// Capture `id` is held at a wrap edge (and no longer will be).
    pub(super) fn held_at_wrap(&mut self, id: u32) -> bool {
        self.held.take() == Some(id)
    }
}

impl App {
    /// The edges where no computer sits that wrap or learning want armed,
    /// beside `armed` (the edges leading to peers). None while the pointer
    /// is locked here or a peer controls this computer. A paired computer
    /// that is offline (or whose monitors aren't known yet) still sits on
    /// its side: that whole side is left alone.
    pub(super) fn free_edges(&self, armed: &[(Edge, Fingerprint)]) -> Vec<(Edge, Kind)> {
        let Some(local) = &self.local else { return Vec::new() };
        if self.locked || self.owner.is_some() {
            return Vec::new();
        }
        let edge = &self.settings.edge;
        let wrap = edge.wrap.sides();
        // Learning looks only while a computer whose place is a guess is online.
        let probe = edge.learn != Learn::Off && self.peers.keys().any(|fp| self.trust.get(fp).is_some_and(Device::learnable));
        let sides: Vec<Side> = Side::ALL.into_iter().filter(|s| probe || wrap.contains(s)).collect();
        if sides.is_empty() {
            return Vec::new();
        }
        let me = self.node.identity().fingerprint;
        let mut taken: Vec<Edge> = armed.iter().map(|(e, _)| *e).collect();
        for d in self.trust.devices() {
            let fp = d.fingerprint();
            let known = fp.and_then(|fp| self.peers.get(&fp)).is_some_and(|p| p.desktop.is_some());
            if !known && fp != Some(me) {
                let side = d.side();
                taken.extend(local.outer_edges(side).map(|r| Edge::of(side, r)));
            }
        }
        let kind = |side| if wrap.contains(&side) { Kind::Wrap } else { Kind::Probe };
        free_edges(local, &sides, &taken).into_iter().map(|e| (e, kind(e.side))).collect()
    }

    /// The armed edge where no computer sits that caught the pointer on
    /// `side` at `pos`, joined with the pieces that continue it in a
    /// straight line (what an edge rule holding it there lets it slide
    /// along), unless an edge leading to a peer is as near.
    fn free_edge_at(&self, side: Side, pos: Pos) -> Option<(Edge, Kind)> {
        let edges = &self.push.edges;
        let near = edges.iter().filter(|(e, _)| e.side == side && e.dist(pos) <= super::EDGE_SLACK);
        let (mut edge, kind) = *near.min_by(|(a, _), (b, _)| a.dist(pos).total_cmp(&b.dist(pos)))?;
        if self.armed_at(side, pos).is_some_and(|(peer, _)| peer.dist(pos) <= edge.dist(pos)) {
            return None;
        }
        while let Some((e, _)) = edges.iter().find(|(e, k)| {
            *k == kind && e.side == side && e.at == edge.at && (e.hi == edge.lo || e.lo == edge.hi)
        }) {
            edge = Edge { lo: edge.lo.min(e.lo), hi: edge.hi.max(e.hi), ..edge };
        }
        Some((edge, kind))
    }

    /// Capture `id` began at an edge where no computer sits: wrap, or
    /// count the push. False if it was not such an edge.
    pub(super) fn on_free_edge(&mut self, id: u32, side: Side, pos: Pos) -> bool {
        let Some((edge, kind)) = self.free_edge_at(side, pos) else { return false };
        // Normally unreachable (nothing is armed then), but a capture
        // already on its way when that changed may still arrive.
        if self.locked || self.owner.is_some() || self.lock_sync.is_locked() {
            tracing::debug!(%side, "edge with no computer: not now (locked, or controlled from elsewhere)");
            self.hand_back(id);
            return true;
        }
        match kind {
            Kind::Probe => {
                self.hand_back(id);
                self.on_push(side);
            }
            Kind::Wrap => match self.approach_start(edge, pos) {
                Some(Start::Cross(p)) => self.wrap_round(id, side, p),
                Some(Start::Refuse(why)) => {
                    tracing::debug!(%side, "not wrapping: {}", why.as_str());
                    self.hand_back(id);
                }
                Some(Start::Wait(a)) => {
                    tracing::debug!(%side, "holding the pointer at the edge before wrapping");
                    self.approach = Some((id, a));
                    self.push.held = Some(id);
                }
                None => self.release_capture(id, None),
            },
        }
        true
    }

    /// Ends capture `id`, which left through our `side` edge at `at`, with
    /// the pointer at the opposite edge. Input it caught is typed here.
    pub(super) fn wrap_round(&mut self, id: u32, side: Side, at: Pos) {
        let Some(local) = &self.local else {
            self.hand_back(id);
            return;
        };
        let to = wrap_point(local, side, at, HOME_INSET);
        tracing::debug!(%side, ?at, ?to, "pointer wraps round");
        self.release_capture(id, Some(to));
        self.handed_back = Some(id);
        self.push.pushes.reset();
    }

    /// A probe caught the pointer on `side`: twice in a row means a
    /// computer is there.
    fn on_push(&mut self, side: Side) {
        let now = Instant::now();
        if !self.push.pushes.hit(side, now) {
            return;
        }
        let Some(act) = learn::act(self.settings.edge.learn, side) else { return };
        let online: Vec<Online<Fingerprint>> = self
            .peers
            .iter()
            .map(|(fp, p)| Online {
                key: *fp,
                side: p.side,
                learnable: self.trust.get(fp).is_some_and(Device::learnable),
                connected: self.push.connected.get(fp).copied().unwrap_or(now),
            })
            .collect();
        let Some(fp) = learn::candidate(&online, side) else { return };
        if !self.push.cooldown.ready(fp, now) {
            tracing::debug!(%side, "pushed twice, but that computer was suggested a moment ago");
            return;
        }
        self.push.cooldown.note(fp, now);
        match act {
            Act::Suggest => self.suggest(fp, side, now),
            Act::Place => self.place_learned(fp, side),
        }
    }

    /// The command-line name of paired computer `fp` (see `device_arg`):
    /// its name only if that finds it and nothing else.
    fn arg_for(&self, fp: Fingerprint) -> String {
        let Some(d) = self.trust.get(&fp) else { return fp.short() };
        let name = sanitize_name(&d.name);
        let unique = self.trust.find(&name).is_some_and(|f| f.fingerprint == d.fingerprint);
        if unique { device_arg(&name, &fp.short()) } else { fp.short() }
    }

    fn suggest(&mut self, fp: Fingerprint, side: Side, now: Instant) {
        self.push.shown = Some(Shown { key: fp, side, at: now });
        let Some(s) = self.suggestion() else { return };
        let title = is_it_there(&s.name, side);
        let body = format!("You pushed the pointer against your {side} edge twice. To place it there, run: {}", s.command());
        println!("{title} {body} (or use the window's Layout tab).");
        tokio::spawn(async move { crate::notify::note(&title, &body).await });
    }

    /// `[edge] learn = auto`: puts `fp` on `side`, through what `one-mk
    /// layout` does: `devices.toml`, then applied here and its mirror sent
    /// to the peer (which adopts it: the version is newer).
    fn place_learned(&mut self, fp: Fingerprint, side: Side) {
        let before = match crate::manage::learn_layout(&self.dir, &fp, side) {
            Ok(Some(before)) => before,
            Ok(None) => return, // no longer a guess: the file changed meanwhile
            Err(e) => {
                tracing::warn!("could not save the learned layout: {e:#}");
                return;
            }
        };
        self.push.shown = None;
        self.reload_trust();
        self.sync_sides();
        self.send_layout(fp);
        let name = self.name_for(fp);
        let undo = format!("one-mk layout {} {}", self.arg_for(fp), before.side());
        let title = format!("{name} is now {}", on_your(side));
        let body = format!("You pushed the pointer against your {side} edge twice, so one-mk placed it there. To undo, run: {undo}");
        // `sync_sides` has said where it is now.
        println!("{body}");
        tokio::spawn(async move { crate::notify::note(&title, &body).await });
    }

    /// The pointer crossed to `fp` through the edge it was guessed (or
    /// learned) to be at: that place is right, and learning leaves it be.
    pub(super) fn confirm_place(&mut self, fp: Fingerprint) {
        self.push.pushes.reset(); // the pointer went elsewhere
        if self.push.shown.is_some_and(|s| s.key == fp) {
            self.push.shown = None;
        }
        if !self.trust.get(&fp).is_some_and(Device::learnable) {
            return; // the usual case: nothing to read or write
        }
        // Saved from what is on disk now, so a change made meanwhile (a
        // `one-mk layout` not yet applied) is not written over.
        self.reload_trust();
        let Some(d) = self.trust.get_mut(&fp).filter(|d| d.learnable()) else { return };
        d.placed_by = PlacedBy::Confirmed;
        if let Err(e) = self.trust.save() {
            tracing::debug!("saving that the layout was used: {e:#}");
        }
        self.update_sides(); // the probes may go
    }

    /// The suggestion to show now, if it is recent and still applies.
    pub(super) fn suggestion(&self) -> Option<Suggestion> {
        let s = self.push.shown.filter(|s| s.current(Instant::now()))?;
        let d = self.trust.get(&s.key).filter(|d| d.learnable() && d.side() != s.side)?;
        Some(Suggestion { id: d.short_id(), name: self.name_for(s.key), side: s.side, arg: self.arg_for(s.key) })
    }

    /// The edge rules in words, for `one-mk status`, saying when wrap or
    /// learning acts with an input backend it wasn't tested with.
    pub(super) fn edges_status(&self) -> String {
        let edge = &self.settings.edge;
        let mut words = edge.describe();
        let acts = edge.wrap != crate::settings::Wrap::Off || edge.learn == Learn::Auto;
        if acts && crate::doctor::edge_push_tested(self.backend_name) == Some(false) {
            words += &format!(" (not yet tested on {})", crate::doctor::backend_words(self.backend_name));
        }
        words
    }

    /// `one-mk edge --dismiss`, the window's "No".
    pub(super) fn dismiss_suggestion(&mut self) -> String {
        let shown = self.suggestion().is_some();
        self.push.shown = None;
        if shown { "ok\n".to_owned() } else { "none\n".to_owned() }
    }
}
