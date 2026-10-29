//! The Layout tab: every computer's real monitors on a flat plan, as the
//! daemon reports them (`Status.layout`). Drag a computer along the edge
//! it shares with this one to line them up (it snaps to the ends and the
//! middle), or round the corner to another side. The part of the edge the
//! pointer crosses is drawn in amber; the rest of that edge is a wall.
//! Under the plan: the chosen computer's side and alignment, and the
//! saved layout profiles. Changes go through `one-mk layout` and
//! `one-mk profile`.

use super::*;
use crate::view::{self, Snap};
use one_mk_proto::{Align, Rect as PRect};

/// Screen = world × scale + off; frozen while dragging so the plan holds still.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Xf {
    scale: f32,
    off: egui::Vec2,
}

impl Xf {
    /// Fits world pixels `min..max` into `area`, centred.
    fn fit(min: (i32, i32), max: (i32, i32), area: Rect) -> Xf {
        let (w, h) = ((max.0 - min.0).max(1) as f32, (max.1 - min.1).max(1) as f32);
        let scale = (area.width() / w).min(area.height() / h).min(0.2);
        let used = vec2(w, h) * scale;
        let off = area.center().to_vec2() - used / 2.0 - vec2(min.0 as f32, min.1 as f32) * scale;
        Xf { scale, off }
    }
    fn pt(&self, x: f64, y: f64) -> Pos2 {
        pos2(x as f32 * self.scale + self.off.x, y as f32 * self.scale + self.off.y)
    }
    fn rect(&self, r: PRect) -> Rect {
        Rect::from_min_max(self.pt(f64::from(r.x), f64::from(r.y)), self.pt(f64::from(r.right()), f64::from(r.bottom())))
    }
}

pub(super) struct PlanDrag {
    id: String,
    start: Pos2,
    /// Where its monitors' bounds started, in world pixels.
    origin: (i32, i32),
    side: Side,
    align: Align,
    xf: Xf,
}

/// One computer as the plan draws it.
#[derive(Clone)]
struct Item {
    id: String,
    name: String,
    online: bool,
    side: Side,
    align: Align,
    /// Its monitors as it reports them (empty: not known).
    real: Vec<PRect>,
    /// Where they go on the plan.
    placed: view::Placed,
    wall: [u32; 2],
}

fn bounds(screens: &[PRect]) -> PRect {
    one_mk_proto::Desktop::new(if screens.is_empty() { vec![view::GUESS] } else { screens.to_vec() }).bounds()
}

fn wall_color(wall: [u32; 2], k: f32) -> Color32 {
    let c = |w: u32| Color32::from_rgb((w >> 16) as u8, (w >> 8) as u8, w as u8);
    mix(c(wall[1]), c(wall[0]), k)
}

impl App {
    /// The side and alignment shown for a computer: while dragged, where
    /// it would land; then what was chosen here and not yet confirmed.
    pub(super) fn place_shown(&self, id: &str, side: Side, align: Align) -> (Side, Align) {
        if let Some(d) = self.plan_drag.as_ref().filter(|d| d.id == id) {
            return (d.side, d.align);
        }
        if let Some((s, a, _)) = self.pending_align.get(id) {
            return (*s, *a);
        }
        match self.pending_side.get(id) {
            // A new side starts stretched, as `one-mk layout` does.
            Some((s, _)) if *s != side => (*s, Align::Stretch),
            _ => (side, align),
        }
    }

    fn my_screens(&self) -> (Vec<PRect>, bool) {
        let s = self.running().map(|s| s.layout.screens.clone()).unwrap_or_default();
        if s.is_empty() { (vec![view::GUESS], true) } else { (s, false) }
    }

    fn plan_items(&self) -> Vec<Item> {
        let Some(s) = self.running() else { return vec![] };
        let (me, _) = self.my_screens();
        s.peers
            .iter()
            .enumerate()
            .map(|(n, p)| {
                let place = s.layout.place(&p.id);
                let real = place.map(|pl| pl.screens.clone()).unwrap_or_default();
                let (side, align) = self.place_shown(&p.id, p.side, place.map_or(Align::Stretch, |pl| pl.align));
                Item {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    online: p.online,
                    side,
                    align,
                    placed: view::place(&me, &real, side, align),
                    real,
                    wall: WALLS[n % WALLS.len()],
                }
            })
            .collect()
    }

    /// The selected computer if it is a paired one.
    fn plan_selected(&self) -> Option<String> {
        self.selected_id().filter(|id| id != "me")
    }

    pub(super) fn plan(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        let area = Rect::from_min_max(pos2(rect.left() + 48.0, rect.top() + TOP_H + 36.0), pos2(rect.right() - 48.0, rect.bottom() - 52.0));
        let (me, me_guessed) = self.my_screens();
        let items = self.plan_items();
        let world: Vec<PRect> = me.iter().copied().chain(items.iter().flat_map(|i| i.placed.screens.iter().copied())).collect();
        let min = (world.iter().map(|r| r.x).min().unwrap_or(0), world.iter().map(|r| r.y).min().unwrap_or(0));
        let max = (world.iter().map(PRect::right).max().unwrap_or(1), world.iter().map(PRect::bottom).max().unwrap_or(1));
        let fitted = Xf::fit(min, max, area);
        // Glide to a new fit (a computer moved to another side), hold still while dragging.
        let xf = match &self.plan_drag {
            Some(d) => d.xf,
            None => {
                let k = |name: &str, v: f32| tween(ui.ctx(), Id::unique(("plan-xf", name)), v, motion::space(0.3), motion::out_cubic);
                Xf { scale: k("s", fitted.scale), off: vec2(k("x", fitted.off.x), k("y", fitted.off.y)) }
            }
        };
        let sel = self.plan_selected();
        let locked = widgets::locked(ui);
        let resp = ui.interact(area.expand(40.0), Id::unique("plan"), if locked { Sense::hover() } else { Sense::click_and_drag() });
        let hit = |p: Pos2| -> Option<usize> {
            // The selected one is drawn on top, so it is found first.
            let order = items.iter().enumerate().filter(|(_, i)| Some(&i.id) == sel.as_ref()).chain(items.iter().enumerate().filter(|(_, i)| Some(&i.id) != sel.as_ref()));
            order.map(|(k, _)| k).find(|&k| items[k].placed.screens.iter().any(|s| xf.rect(*s).expand(2.0).contains(p)))
        };
        let on_me = |p: Pos2| me.iter().any(|s| xf.rect(*s).contains(p));
        let hover = resp.hover_pos().filter(|_| !locked).and_then(hit);

        self.plan_keys(ui, &items, &xf, now);

        // Development aid: ONE_MK_APP_PLAN_DRAG=PX carries the chosen
        // computer PX pixels along its edge, and holds it there.
        #[cfg(debug_assertions)]
        if let Some(px) = std::env::var("ONE_MK_APP_PLAN_DRAG").ok().and_then(|v| v.parse::<f32>().ok())
            && now > 1.2
            && let Some(it) = items.iter().find(|i| Some(&i.id) == sel.as_ref())
        {
            let rb = bounds(&it.real);
            let (ll, rl) = view::edge_lengths(bounds(&me), rb, it.side);
            let base = view::offset_px(self.running().and_then(|s| s.layout.place(&it.id)).map_or(Align::Stretch, |p| p.align), ll, rl);
            let t = ((now - 1.2) / 0.8).min(1.0) as f32;
            let b = bounds(&it.placed.screens);
            let d = self.plan_drag.get_or_insert(PlanDrag { id: it.id.clone(), start: Pos2::ZERO, origin: (b.x, b.y), side: it.side, align: it.align, xf });
            d.align = view::snap_offset(f64::from(base as f32 + px * t), ll, rl, f64::from(10.0 / d.xf.scale));
            ui.ctx().request_repaint();
        }
        if resp.drag_started()
            && let Some(p) = resp.interact_pointer_pos()
            && let Some(k) = hit(p)
        {
            let it = &items[k];
            let b = bounds(&it.placed.screens);
            self.selected = Some(it.id.clone());
            self.plan_drag = Some(PlanDrag { id: it.id.clone(), start: p, origin: (b.x, b.y), side: it.side, align: it.align, xf });
        }
        if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.dragged())
            && let Some(d) = &mut self.plan_drag
        {
            let item = items.iter().find(|i| i.id == d.id);
            if let Some(it) = item {
                let delta = (p - d.start) / d.xf.scale;
                let rb = bounds(&it.real);
                let lb = bounds(&me);
                let (x, y) = (d.origin.0 as f32 + delta.x, d.origin.1 as f32 + delta.y);
                // Which side it is nearest, measured in half-sizes so a
                // wide screen doesn't pull everything to the top.
                let cx = (x + rb.w as f32 / 2.0 - (lb.x as f32 + lb.w as f32 / 2.0)) / ((lb.w + rb.w) as f32 / 2.0);
                let cy = (y + rb.h as f32 / 2.0 - (lb.y as f32 + lb.h as f32 / 2.0)) / ((lb.h + rb.h) as f32 / 2.0);
                let side = if cx.abs() >= cy.abs() {
                    if cx < 0.0 { Side::Left } else { Side::Right }
                } else if cy < 0.0 {
                    Side::Top
                } else {
                    Side::Bottom
                };
                let (ll, rl) = view::edge_lengths(lb, rb, side);
                let raw = if side.along_x() { x - lb.x as f32 } else { y - lb.y as f32 };
                d.side = side;
                d.align = view::snap_offset(f64::from(raw), ll, rl, f64::from(10.0 / d.xf.scale));
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if resp.drag_stopped()
            && let Some(d) = self.plan_drag.take()
        {
            let was = items.iter().find(|i| i.id == d.id).map(|i| {
                let real = self.running().and_then(|s| s.peers.iter().find(|p| p.id == i.id)).map(|p| p.side);
                (real, i.align)
            });
            if was.is_some_and(|(side, align)| side != Some(d.side) || align != d.align) {
                self.choose_align(&d.id, d.side, d.align);
            }
        }
        if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            match hit(p) {
                Some(k) => self.selected = Some(items[k].id.clone()),
                None if on_me(p) => self.selected = Some("me".into()),
                None => {}
            }
        }
        if self.plan_drag.is_none() && hover.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        // Draw: the edges' walls, this computer, the others, then the
        // shared parts of the edges on top.
        let p = ui.painter_at(rect);
        let sel = self.plan_selected();
        let lb = bounds(&me);
        let edge_line = |side: Side, a: f64, b: f64| -> [Pos2; 2] {
            let (l, r, t, bo) = (f64::from(lb.x), f64::from(lb.right()), f64::from(lb.y), f64::from(lb.bottom()));
            match side {
                Side::Left => [xf.pt(l, a), xf.pt(l, b)],
                Side::Right => [xf.pt(r, a), xf.pt(r, b)],
                Side::Top => [xf.pt(a, t), xf.pt(b, t)],
                Side::Bottom => [xf.pt(a, bo), xf.pt(b, bo)],
            }
        };
        let kb = widgets::keyboard_focus(ui.ctx());
        let focused = self.plan_focus.clone();
        let mut seams = vec![];
        for it in &items {
            let (shared, walls) = view::seam_parts(&me, &it.real, it.side, it.align);
            seams.push((it.side, shared, walls, Some(&it.id) == sel.as_ref()));
        }
        let draw_computer = |it_screens: &[PRect], name: &str, sub: &str, wall: [u32; 2], guessed: bool, ring: Option<Color32>, dim: bool| {
            let largest = it_screens.iter().max_by_key(|s| i64::from(s.w) * i64::from(s.h)).copied();
            for s in it_screens {
                let r = xf.rect(*s).shrink(1.0);
                if guessed {
                    p.rect_filled(r, CornerRadius::same(5), PANEL);
                    p.add(egui::Shape::dashed_line(&[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()], Stroke::new(1.0, OFF), 5.0, 4.0));
                } else {
                    // A quiet wallpaper: lighter at the top, as on the desk.
                    let mut m = egui::Mesh::default();
                    let (top, bottom) = (wall_color(wall, 0.85), wall_color(wall, 0.15));
                    for (pt, c) in [(r.left_top(), top), (r.right_top(), top), (r.right_bottom(), bottom), (r.left_bottom(), bottom)] {
                        m.colored_vertex(pt, if dim { mix(c, GROUND, 0.45) } else { c });
                    }
                    m.add_triangle(0, 1, 2);
                    m.add_triangle(0, 2, 3);
                    p.add(egui::Shape::mesh(m));
                    p.rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, LINE), StrokeKind::Inside);
                }
                if let Some(c) = ring {
                    p.rect_stroke(r, CornerRadius::same(5), Stroke::new(1.5, c), StrokeKind::Outside);
                }
                let size = format!("{} × {}", s.w, s.h);
                if Some(*s) != largest && text_width(ui, &size, mono(T_META)) + 12.0 < r.width() && r.height() > 30.0 {
                    p.text(r.center(), Align2::CENTER_CENTER, size, mono(T_META), INK2);
                }
            }
            if let Some(big) = largest {
                let r = xf.rect(big);
                let max_w = r.width() - 16.0;
                if max_w > 24.0 {
                    // The second line only if it fits whole: a cut number misleads.
                    let sub_font = if sub.contains('×') { mono(T_META) } else { regular(T_META) };
                    let two = r.height() > 44.0 && text_width(ui, sub, sub_font) <= max_w;
                    let y = if two { r.center().y - 9.0 } else { r.center().y };
                    let col = if dim { INK2 } else { INK };
                    p.text(pos2(r.center().x, y), Align2::CENTER_CENTER, fit(&p, name, &semibold(T_BODY), max_w), semibold(T_BODY), col);
                    if two {
                        let font = if sub.contains('×') { mono(T_META) } else { regular(T_META) };
                        p.text(pos2(r.center().x, y + 19.0), Align2::CENTER_CENTER, fit(&p, sub, &font, max_w), font, INK2);
                    }
                }
            }
        };
        for (side, _, walls, chosen) in &seams {
            for &(a, b) in walls {
                let col = if *chosen { INK2 } else { OFF };
                p.add(egui::Shape::dashed_line(&edge_line(*side, a, b), Stroke::new(2.0, col), 4.0, 4.0));
            }
        }
        let me_name = self.running().map_or_else(String::new, |s| s.name.clone());
        let me_sub = if me_guessed { "This computer · size not known yet".to_owned() } else { "This computer".to_owned() };
        let me_ring = (self.selected.as_deref() == Some("me")).then_some(INK_HOVER);
        draw_computer(&me, &me_name, &me_sub, ME_WALL, false, me_ring, false);
        let mut order: Vec<&Item> = items.iter().filter(|i| Some(&i.id) != sel.as_ref()).collect();
        order.extend(items.iter().filter(|i| Some(&i.id) == sel.as_ref()));
        for it in order {
            let chosen = Some(&it.id) == sel.as_ref();
            let hovered = hover.is_some_and(|k| items[k].id == it.id);
            let ring = if chosen {
                Some(INK)
            } else if hovered || (kb && focused.as_ref() == Some(&it.id)) {
                Some(INK2)
            } else {
                None
            };
            let big = it.placed.screens.iter().max_by_key(|s| i64::from(s.w) * i64::from(s.h)).copied().unwrap_or(view::GUESS);
            let sub = if !it.online {
                "Offline".to_owned()
            } else if it.placed.guessed {
                "Size not known yet".to_owned()
            } else {
                format!("{} × {}", big.w, big.h)
            };
            draw_computer(&it.placed.screens, &it.name, &sub, it.wall, it.placed.guessed, ring, !it.online);
            if kb && focused.as_ref() == Some(&it.id) {
                let b = xf.rect(bounds(&it.placed.screens));
                p.rect_stroke(b.expand(3.0), CornerRadius::same(8), Stroke::new(2.0, SIGNAL), StrokeKind::Outside);
            }
        }
        for (side, shared, _, _) in &seams {
            for &(a, b) in shared {
                p.line_segment(edge_line(*side, a, b), Stroke::new(3.0, SIGNAL));
            }
        }
        // While dragging: where it snaps to, in words, beside it.
        if let Some(d) = &self.plan_drag
            && let Some(it) = items.iter().find(|i| i.id == d.id)
        {
            let lb2 = bounds(&me);
            let rb = bounds(&it.real);
            let (ll, rl) = view::edge_lengths(lb2, rb, d.side);
            let text = match view::snap_of(d.align, ll, rl) {
                Some(s) => format!("{} · {}", side_title(d.side), view::snap_names(d.side)[s as usize]),
                None => format!("{} · {:+} px", side_title(d.side), view::offset_px(d.align, ll, rl)),
            };
            let b = xf.rect(bounds(&it.placed.screens));
            let tw = text_width(ui, &text, mono(T_META));
            // Beside it, away from this computer, inside the plan.
            let size = vec2(tw + 18.0, 22.0);
            let at = match d.side {
                Side::Left => pos2(b.left() - 12.0 - size.x / 2.0, b.center().y),
                Side::Top => pos2(b.center().x, b.top() - 18.0),
                Side::Right | Side::Bottom => pos2(b.right() + 12.0 + size.x / 2.0, b.center().y),
            };
            let room = rect.shrink(8.0);
            let at = pos2(at.x.clamp(room.left() + size.x / 2.0, room.right() - size.x / 2.0), at.y.clamp(room.top() + TOP_H, room.bottom() - 40.0));
            let tag = Rect::from_center_size(at, size);
            p.rect(tag, CornerRadius::same(6), RAISED, Stroke::new(1.0, LINE), StrokeKind::Inside);
            p.text(tag.center(), Align2::CENTER_CENTER, text, mono(T_META), INK);
        }
        // A short key under the plan.
        let key = pos2(area.left(), rect.bottom() - 24.0);
        p.line_segment([key, key + vec2(18.0, 0.0)], Stroke::new(3.0, SIGNAL));
        let tx = p.text(key + vec2(26.0, 0.0), Align2::LEFT_CENTER, "The pointer crosses here", regular(T_META), INK2).right();
        let wall = key + vec2(tx - key.x + 22.0, 0.0);
        p.add(egui::Shape::dashed_line(&[wall, wall + vec2(18.0, 0.0)], Stroke::new(2.0, OFF), 4.0, 4.0));
        p.text(wall + vec2(26.0, 0.0), Align2::LEFT_CENTER, "Wall", regular(T_META), INK2);
        let hint = if items.is_empty() { "Pair a computer to arrange it here." } else { "Drag a computer along an edge to line it up." };
        p.text(pos2(area.right(), rect.bottom() - 24.0), Align2::RIGHT_CENTER, hint, regular(T_META), INK2);
    }

    /// Keyboard stops for the computers on the plan; arrows move the
    /// focused one along its edge (Shift: further), Enter chooses it.
    fn plan_keys(&mut self, ui: &mut Ui, items: &[Item], xf: &Xf, now: f64) {
        let mut focused = None;
        for it in items {
            let b = xf.rect(bounds(&it.placed.screens));
            let r = ui.interact(b, Id::unique(("plan-device", &it.id)), widgets::sense(ui, Sense::focusable_noninteractive()));
            let chosen = self.selected.as_deref() == Some(it.id.as_str());
            let label = format!("{}, {}. Arrow keys move it along the edge.", it.name, view::placement_sentence(it.side, it.align));
            r.widget_info(move || egui::WidgetInfo::selected(Role::Button, true, chosen, &label));
            if r.has_focus() {
                focused = Some(it.id.clone());
            }
        }
        self.plan_focus = focused.clone();
        let Some(id) = focused else { return };
        let Some(it) = items.iter().find(|i| i.id == id) else { return };
        let step = if ui.input(|i| i.modifiers.shift) { 100 } else { 10 };
        let (back, fwd) = if it.side.along_x() { (Key::ArrowLeft, Key::ArrowRight) } else { (Key::ArrowUp, Key::ArrowDown) };
        let delta = ui.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, back) || i.consume_key(Modifiers::SHIFT, back) {
                -step
            } else if i.consume_key(Modifiers::NONE, fwd) || i.consume_key(Modifiers::SHIFT, fwd) {
                step
            } else {
                0
            }
        });
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            self.selected = Some(id.clone());
        }
        if delta != 0 {
            let (me, _) = self.my_screens();
            let (ll, rl) = view::edge_lengths(bounds(&me), bounds(&it.real), it.side);
            let o = view::offset_px(it.align, ll, rl) + delta;
            let align = view::snap_offset(f64::from(o), ll, rl, 0.0);
            self.selected = Some(id.clone());
            // Shown at once; applied once the keys rest.
            self.pending_align.insert(id.clone(), (it.side, align, now));
            self.nudge = Some((id, now + 0.6));
            ui.ctx().request_repaint_after_secs(0.6);
        }
    }

    /// Applies an arrow-key nudge once the keys have rested.
    pub(super) fn settle_nudge(&mut self, now: f64) {
        let Some((id, at)) = self.nudge.clone() else { return };
        if now < at {
            self.ctx.request_repaint_after_secs((at - now) as f32);
            return;
        }
        self.nudge = None;
        if let Some((side, align, _)) = self.pending_align.get(&id).copied() {
            self.choose_align(&id, side, align);
        }
    }

    pub(super) fn choose_align(&mut self, id: &str, side: Side, align: Align) {
        let name = self.name_of(id);
        self.pending_align.insert(id.to_owned(), (side, align, self.now));
        self.busy.push(Job::Align { id: id.into(), name: name.clone(), side, align });
        self.link.set_align(id, &name, side, align);
        self.sync_scene();
    }

    /// The chosen computer's side, in words and as a control.
    pub(super) fn arrange_cell(&mut self, ui: &mut Ui, c: Rect, sel: Option<String>) {
        let Some(status) = self.running().cloned() else { return };
        let Some(peer) = status.peers.iter().find(|p| Some(&p.id) == sel.as_ref()).cloned() else {
            let room = chip(ui, c, "This computer", None);
            title_row(ui, c, &status.name, room);
            let (me, guessed) = self.my_screens();
            let b = bounds(&me);
            let text = match (guessed, me.len()) {
                (true, _) => "Its screen size isn't known yet: one·mk learns it once input sharing has started.".to_owned(),
                (false, 1) => format!("One monitor, {} × {}.", b.w, b.h),
                (false, n) => format!("{n} monitors, {} × {} in all.", b.w, b.h),
            };
            let h = para(ui, pos2(c.left(), c.top() + 30.0), c.width(), &text, regular(T_BODY), INK2);
            let more = if status.peers.is_empty() { "Pair a computer to place it around this one." } else { "Choose a computer on the plan to place it." };
            para(ui, pos2(c.left(), c.top() + 30.0 + h + 6.0), c.width(), more, regular(T_BODY), INK2);
            a11y_text(ui, Id::unique("arrange-me"), c, Role::Label, &format!("{}. {text} {more}", status.name), None);
            return;
        };
        let room = if peer.online { chip(ui, c, "Connected", Some(true)) } else { chip(ui, c, "Offline", Some(false)) };
        title_row(ui, c, &peer.name, room);
        let place = status.layout.place(&peer.id);
        let (side, align) = self.place_shown(&peer.id, peer.side, place.map_or(Align::Stretch, |p| p.align));
        let track = Rect::from_min_size(pos2(c.left(), c.top() + 28.0), vec2(c.width(), 26.0));
        let labels: Vec<&str> = PLACES.iter().map(|(_, l)| *l).collect();
        let pills = Pills {
            id: Id::unique(("plan-position", &peer.id)),
            track,
            cells: Pills::even(track, 4, 2.0),
            labels: &labels,
            names: PLACES.iter().map(|(_, l)| format!("Position of {}: {l}", peer.name)).collect(),
            chosen: PLACES.iter().position(|(s, _)| *s == side),
            enabled: vec![true; 4],
            role: Role::RadioButton,
            font: if c.width() >= 230.0 { T_BODY } else { T_META },
            radius: 6,
        };
        if let Some(k) = pills.show(ui, self.arrowed) {
            self.pending_align.remove(&peer.id);
            self.choose_side(&peer.id, PLACES[k].0, true);
        }
        let sentence = view::placement_sentence(side, align);
        let mut y = c.top() + 66.0;
        y += para(ui, pos2(c.left(), y), c.width(), &sentence, regular(T_BODY), INK2) + 6.0;
        a11y_text(ui, Id::unique(("arrange-sentence", &peer.id)), Rect::from_min_size(pos2(c.left(), c.top() + 66.0), vec2(c.width(), 20.0)), Role::Label, &sentence, Some(Live::Polite));
        // Two computers claiming the same stretch of one edge.
        let (me, _) = self.my_screens();
        let mine = view::seam_parts(&me, &place.map(|p| p.screens.clone()).unwrap_or_default(), side, align).0;
        let clash = status.peers.iter().filter(|o| o.id != peer.id).find(|o| {
            let op = status.layout.place(&o.id);
            let (os, oa) = self.place_shown(&o.id, o.side, op.map_or(Align::Stretch, |p| p.align));
            os == side && one_mk_proto::overlaps(&mine, &view::seam_parts(&me, &op.map(|p| p.screens.clone()).unwrap_or_default(), os, oa).0)
        });
        let warning = if mine.is_empty() {
            Some("The edges don't meet: the pointer can't cross to it. Drag it along the edge until they touch.".to_owned())
        } else {
            clash.map(|o| format!("{} shares part of this edge; the pointer goes to whichever was placed first.", o.name))
        };
        if let Some(text) = warning {
            p_dot(ui, pos2(c.left() + 3.0, y + 9.0), ERROR);
            para(ui, pos2(c.left() + 14.0, y), c.width() - 14.0, &text, regular(T_META), INK);
            a11y_text(ui, Id::unique(("arrange-warning", &peer.id)), Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), 30.0)), Role::Alert, &text, None);
        }
    }

    /// Stretched, or lined up at an end, the middle or an exact offset.
    pub(super) fn line_up_cell(&mut self, ui: &mut Ui, c: Rect, sel: Option<String>) {
        let Some(status) = self.running().cloned() else { return };
        let Some(peer) = status.peers.iter().find(|p| Some(&p.id) == sel.as_ref()).cloned() else {
            para(ui, c.left_top(), c.width(), "Line up edges", semibold(T_TITLE), INK);
            let text = "Choose a computer to line its edge up with this one's: stretched end to end, or 1:1 like two monitors side by side.";
            para(ui, c.left_top() + vec2(0.0, 26.0), c.width(), text, regular(T_BODY), INK2);
            return;
        };
        let place = status.layout.place(&peer.id);
        let (side, align) = self.place_shown(&peer.id, peer.side, place.map_or(Align::Stretch, |p| p.align));
        let (me, _) = self.my_screens();
        let (ll, rl) = view::edge_lengths(bounds(&me), bounds(&place.map(|p| p.screens.clone()).unwrap_or_default()), side);
        let stretch = align == Align::Stretch;
        let top = c.top() - 8.0;
        let r0 = Rect::from_min_size(pos2(c.left(), top), vec2(c.width(), 36.0));
        if switch(ui, r0, Id::unique(("stretch", &peer.id)), "Stretch to fit", stretch, true).clicked() {
            self.choose_align(&peer.id, side, if stretch { Align::Center } else { Align::Stretch });
        }
        ui.painter().hline(c.x_range(), top + 36.0, Stroke::new(1.0, LINE));
        let names = view::snap_names(side);
        let track = Rect::from_min_size(pos2(c.left(), top + 46.0), vec2(c.width(), 26.0));
        let pills = Pills {
            id: Id::unique(("snap", &peer.id)),
            track,
            cells: Pills::even(track, 3, 2.0),
            labels: &names,
            names: names.iter().map(|n| format!("Line up {} at the {}", peer.name, n.to_lowercase())).collect(),
            chosen: view::snap_of(align, ll, rl).map(|s| s as usize),
            enabled: vec![!stretch; 3],
            role: Role::RadioButton,
            font: T_BODY,
            radius: 6,
        };
        if let Some(k) = pills.show(ui, self.arrowed) {
            let snap = [Snap::Start, Snap::Middle, Snap::End][k];
            self.choose_align(&peer.id, side, view::snap_align(snap, ll, rl));
        }
        let y = top + 84.0;
        ui.painter().text(pos2(c.left(), y + 14.0), Align2::LEFT_CENTER, "Offset", regular(T_BODY), if stretch { INK2 } else { INK });
        let fw = 92.0;
        let unit_w = text_width(ui, "px", regular(T_META)) + 8.0;
        let fr = Rect::from_min_size(pos2(c.right() - fw - unit_w, y), vec2(fw, 28.0));
        ui.painter().text(pos2(c.right(), y + 14.0), Align2::RIGHT_CENTER, "px", regular(T_META), INK2);
        let current = if stretch { String::new() } else { view::offset_px(align, ll, rl).to_string() };
        let (from, to) = if side.along_x() { ("left", "left") } else { ("top", "top") };
        let label = format!("Offset of {}'s {to} edge from this computer's {from} edge, in pixels", peer.name);
        if let Some(text) = self.edit_field(ui, fr, &format!("offset-{}", peer.id), &current, if stretch { "—" } else { "0" }, &label, !stretch, true) {
            match text.trim_end_matches("px").trim().parse::<i32>() {
                Ok(o) => {
                    let a = view::snap_offset(f64::from(o), ll, rl, 0.0);
                    self.choose_align(&peer.id, side, a);
                }
                Err(_) => self.toast_error(format!("“{text}” isn't a number of pixels."), None, self.now),
            }
        }
        let hint = if stretch {
            "Every point of your edge leads there, scaled to fit."
        } else if side.along_x() {
            "From your left edge to its left edge. 1:1, like two monitors."
        } else {
            "From your top edge to its top edge. 1:1, like two monitors."
        };
        para(ui, pos2(c.left(), y + 38.0), c.width(), hint, regular(T_META), INK2);
    }

    /// Saved layouts: use one, save this one, delete one, or let one switch
    /// in by itself on a Wi-Fi network.
    pub(super) fn profiles_cell(&mut self, ui: &mut Ui, c: Rect) {
        let Some(status) = self.running().cloned() else { return };
        let current = status.layout.profile.clone();
        let names = status.layout.profiles.clone();
        let room = match &current {
            Some(p) => chip(ui, c, &fit(ui.painter(), &format!("In use: {p}"), &regular(T_META), c.width() * 0.55), Some(true)),
            None => 0.0,
        };
        title_row(ui, c, "Profiles", room);
        let save_y = c.bottom() - 28.0;
        // Saving the layout as it is now, under a name.
        let bw = button_width(ui, "Save") - 4.0;
        let fr = Rect::from_min_size(pos2(c.left(), save_y), vec2(c.width() - bw - 8.0, 28.0));
        let name = self.drafts.get("profile-new").cloned().unwrap_or_default();
        let mut text = name.clone();
        let fid = Id::unique(("field", "profile-new"));
        let r = widgets::field(ui, fr, fid, &mut text, "Name this layout", "Name for a new profile of this layout", true, false);
        self.drafts.insert("profile-new".into(), text.clone());
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let b = Rect::from_min_size(pos2(c.right() - bw, save_y), vec2(bw, 28.0));
        // Nothing paired: there is no layout to save yet.
        let can = !text.trim().is_empty() && !status.peers.is_empty();
        if (button(ui, b, Id::unique("profile-save"), "Save", Look::Quiet, can, Some("Save this layout as a profile")).clicked() || enter) && can {
            let n = text.trim().to_owned();
            self.drafts.remove("profile-new");
            self.busy.push(Job::Profile { verb: "save", name: n.clone() });
            self.link.profile("save", &n);
        }
        let list = Rect::from_min_max(pos2(c.left(), c.top() + 26.0), pos2(c.right(), save_y - 6.0));
        if names.is_empty() {
            let text = if status.peers.is_empty() {
                "Once computers are paired, save where they are to switch back in one click, or by itself on a Wi-Fi network."
            } else {
                "Save where every computer is now, to switch back in one click, or by itself on a Wi-Fi network."
            };
            para(ui, list.min + vec2(0.0, 2.0), list.width(), text, regular(T_META), INK2);
            return;
        }
        let mut child = ui.new_child(UiBuilder::new().max_rect(list.expand2(vec2(4.0, 0.0))));
        child.set_clip_rect(list.expand2(vec2(6.0, 0.0)));
        let mut act: Option<(&'static str, String)> = None;
        let mut auto: Option<(String, Option<String>)> = None;
        let mut toggle_open = None;
        let open = self.profile_open.clone();
        let just_opened = std::mem::take(&mut self.profile_scroll);
        egui::ScrollArea::vertical().id_salt("profiles").auto_shrink([false, false]).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for n in &names {
                let expanded = open.as_ref() == Some(n);
                let h = if expanded { 30.0 + 36.0 } else { 30.0 };
                let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
                if expanded && just_opened {
                    ui.scroll_to_rect(row, None);
                }
                let row = row.shrink2(vec2(4.0, 0.0));
                let line = Rect::from_min_size(row.min, vec2(row.width(), 30.0));
                let p = ui.painter().clone();
                let in_use = current.as_ref() == Some(n);
                let auto_text = self.prefs.auto.get(n).cloned();
                let more = Rect::from_min_size(pos2(line.right() - 26.0, line.center().y - 13.0), vec2(26.0, 26.0));
                let use_w = button_width(ui, "In use") - 6.0;
                let use_r = Rect::from_min_size(pos2(more.left() - 6.0 - use_w, line.center().y - 13.0), vec2(use_w, 26.0));
                let text_w = use_r.left() - line.left() - 10.0;
                let shown = fit(&p, n, &regular(T_BODY), text_w);
                let nw = text_width(ui, &shown, regular(T_BODY));
                p.text(pos2(line.left(), line.center().y), Align2::LEFT_CENTER, &shown, regular(T_BODY), INK);
                if shown != *n {
                    ui.interact(Rect::from_min_size(line.min, vec2(nw, 30.0)), Id::unique(("profile-name", n)), Sense::hover()).on_hover_text(n);
                }
                if let Some(t) = auto_text.as_ref().filter(|_| text_w - nw > 40.0) {
                    let tag = if expanded { format!("· automatic on {t}") } else { "· automatic".to_owned() };
                    p.text(pos2(line.left() + nw + 8.0, line.center().y), Align2::LEFT_CENTER, fit(&p, &tag, &regular(T_META), text_w - nw - 8.0), regular(T_META), INK2);
                }
                let label = if in_use { "In use" } else { "Use" };
                if button(ui, use_r, Id::unique(("profile-use", n)), label, Look::Quiet, !in_use, Some(&format!("Use profile {n}"))).clicked() {
                    act = Some(("use", n.clone()));
                }
                let name = if expanded { format!("Close options for {n}") } else { format!("More for profile {n}") };
                if widgets::icon_button(ui, more, Id::unique(("profile-more", n)), &name, true, widgets::draw_dots).clicked() {
                    toggle_open = Some(n.clone());
                }
                if expanded {
                    // One line: a Wi-Fi name to switch to it by itself, and deleting it.
                    let y = line.bottom() + 2.0;
                    let bw = |l: &str| button_width(ui, l) - 6.0;
                    let mut x = row.right();
                    let mut place = |w: f32| {
                        x -= w;
                        let r = Rect::from_min_size(pos2(x, y), vec2(w, 28.0));
                        x -= 6.0;
                        r
                    };
                    let del = place(bw("Delete"));
                    let off = auto_text.is_some().then(|| place(bw("Off")));
                    let set = place(bw("Set"));
                    let fr = Rect::from_min_max(pos2(row.left(), y), pos2(x, y + 28.0));
                    let key = format!("ssid-{n}");
                    let mut ssid = self.drafts.get(&key).cloned().unwrap_or_default();
                    let hint = if auto_text.is_some() { "Another Wi-Fi name" } else { "Wi-Fi name" };
                    let r = widgets::field(ui, fr, Id::unique(("field", &key)), &mut ssid, hint, &format!("Wi-Fi network that switches to {n} by itself"), true, false);
                    self.drafts.insert(key.clone(), ssid.clone());
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    let ok = !ssid.trim().is_empty();
                    if (button(ui, set, Id::unique(("profile-auto", n)), "Set", Look::Quiet, ok, Some(&format!("Switch to {n} by itself on this Wi-Fi"))).clicked() || enter) && ok {
                        auto = Some((n.clone(), Some(ssid.trim().to_owned())));
                        self.drafts.remove(&key);
                    }
                    if let Some(or) = off
                        && button(ui, or, Id::unique(("profile-auto-off", n)), "Off", Look::Quiet, true, Some(&format!("Stop switching to {n} by itself"))).clicked()
                    {
                        auto = Some((n.clone(), None));
                    }
                    if button(ui, del, Id::unique(("profile-del", n)), "Delete", Look::Quiet, true, Some(&format!("Delete profile {n}"))).clicked() {
                        act = Some(("delete", n.clone()));
                    }
                }
                p.hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, LINE));
            }
        });
        if let Some(n) = toggle_open {
            self.profile_open = if self.profile_open.as_ref() == Some(&n) { None } else { Some(n) };
            self.profile_scroll = self.profile_open.is_some();
        }
        if let Some((verb, n)) = act {
            if verb == "delete" {
                self.profile_open = None;
            }
            self.busy.push(Job::Profile { verb, name: n.clone() });
            self.link.profile(verb, &n);
        }
        if let Some((n, ssid)) = auto {
            self.busy.push(Job::ProfileAuto { name: n.clone(), ssid: ssid.clone() });
            self.link.profile_auto(&n, ssid);
        }
    }
}

/// "Left", "Right", "Above", "Below".
fn side_title(side: Side) -> &'static str {
    PLACES.iter().find(|(s, _)| *s == side).map_or("", |(_, l)| l)
}

/// A small status dot.
pub(super) fn p_dot(ui: &Ui, at: Pos2, col: Color32) {
    ui.painter().circle_filled(at, 3.0, col);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_fits_and_centres() {
        let area = Rect::from_min_size(pos2(100.0, 50.0), vec2(400.0, 200.0));
        let xf = Xf::fit((0, 0), (4000, 1000), area);
        assert!((xf.scale - 0.1).abs() < 1e-6, "the width decides: {}", xf.scale);
        let r = xf.rect(PRect::new(0, 0, 4000, 1000));
        assert!((r.center() - area.center()).length() < 0.01);
        assert!(area.expand(0.01).contains_rect(r));
        // Never larger than a fifth: one small screen doesn't fill the window.
        assert!(Xf::fit((0, 0), (100, 100), area).scale <= 0.2);
    }
}
