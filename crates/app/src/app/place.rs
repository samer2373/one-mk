//! The Layout tab's panel. The desk above it is the layout editor (drag a
//! computer beside one of this computer's monitors: it then crosses only
//! where their screens touch, see `App::placing`); here are the chosen
//! computer's side, its exact place, and the saved layout profiles.
//! Changes go through `one-mk layout` and `one-mk profile`.

use super::*;

impl App {
    /// The layout warnings the daemon has about `name`.
    fn layout_warnings(status: &Status, name: &str) -> Vec<String> {
        status.layout.warnings.iter().filter(|w| w.contains(name)).cloned().collect()
    }

    /// The chosen computer's side, in words and as a control.
    pub(super) fn arrange_cell(&mut self, ui: &mut Ui, c: Rect, sel: Option<String>) {
        let Some(status) = self.running().cloned() else { return };
        let Some(peer) = status.peers.iter().find(|p| Some(&p.id) == sel.as_ref()).cloned() else {
            let room = chip(ui, c, "This computer", None);
            title_row(ui, c, &status.name, room);
            let b = one_mk_proto::Desktop::new(status.screens.clone());
            let text = match status.screens.len() {
                0 => "Its screen size isn't known yet: one·mk learns it once input sharing has started.".to_owned(),
                1 => format!("One monitor, {} × {}.", b.bounds().w, b.bounds().h),
                n => format!("{n} monitors, {} × {} in all.", b.bounds().w, b.bounds().h),
            };
            let h = para(ui, pos2(c.left(), c.top() + 30.0), c.width(), &text, regular(T_BODY), INK2);
            let more = if status.peers.is_empty() { "Pair a computer to place it around this one." } else { "Choose a computer on the desk to place it." };
            para(ui, pos2(c.left(), c.top() + 30.0 + h + 6.0), c.width(), more, regular(T_BODY), INK2);
            a11y_text(ui, Id::unique("arrange-me"), c, Role::Label, &format!("{}. {text} {more}", status.name), None);
            return;
        };
        let room = if peer.online { chip(ui, c, "Connected", Some(true)) } else { chip(ui, c, "Offline", Some(false)) };
        title_row(ui, c, &peer.name, room);
        let (side, offset) = self.place_shown(&peer.id, (peer.side, status.offset_of(&peer.id)));
        let track = Rect::from_min_size(pos2(c.left(), c.top() + 28.0), vec2(c.width(), 26.0));
        let labels: Vec<&str> = PLACES.iter().map(|(_, l)| *l).collect();
        let pills = Pills {
            id: Id::unique(("layout-position", &peer.id)),
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
            self.choose_side(&peer.id, PLACES[k].0, true);
        }
        let apart = self.apart.contains(&peer.id);
        let sentence = match offset {
            None => format!("{} is {}: the pointer crosses anywhere along that edge, scaled to fit.", peer.name, on_your(side)),
            Some(_) if apart => format!("{} is {}, placed apart from your monitors.", peer.name, on_your(side)),
            Some(_) => format!("{} is {}, beside your monitors: the pointer crosses where its screens touch yours.", peer.name, on_your(side)),
        };
        let mut y = c.top() + 66.0;
        y += para(ui, pos2(c.left(), y), c.width(), &sentence, regular(T_BODY), INK2) + 6.0;
        a11y_text(ui, Id::unique(("arrange-sentence", &peer.id)), Rect::from_min_size(pos2(c.left(), c.top() + 66.0), vec2(c.width(), 20.0)), Role::Label, &sentence, Some(Live::Polite));
        // The daemon's warnings (two computers on one stretch of edge,
        // screens that don't touch), and screens drawn apart here.
        // The daemon's words are for a terminal; say it shortly here.
        let warnings = Self::layout_warnings(&status, &peer.name);
        let text = if apart || warnings.iter().any(|w| w.contains("does not overlap")) {
            Some("None of its screens touches yours, so the pointer can't cross to it. Drag it next to a monitor.")
        } else if !warnings.is_empty() {
            Some("It shares part of an edge with another computer; the pointer goes to the one placed first.")
        } else {
            None
        };
        if let Some(text) = text {
            p_dot(ui, pos2(c.left() + 3.0, y + 9.0), ERROR);
            para(ui, pos2(c.left() + 14.0, y), c.width() - 14.0, text, regular(T_META), INK);
            a11y_text(ui, Id::unique(("arrange-warning", &peer.id)), Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), 30.0)), Role::Alert, text, None);
        }
    }

    /// Where exactly the chosen computer is, and going back to anywhere
    /// along its side.
    pub(super) fn line_up_cell(&mut self, ui: &mut Ui, c: Rect, sel: Option<String>) {
        let Some(status) = self.running().cloned() else { return };
        para(ui, c.left_top(), c.width(), "Exact place", semibold(T_TITLE), INK);
        let Some(peer) = status.peers.iter().find(|p| Some(&p.id) == sel.as_ref()).cloned() else {
            let text = "Drag a computer next to one of your monitors on the desk. It then crosses only where its screens touch yours, at the same scale, like another monitor.";
            para(ui, c.left_top() + vec2(0.0, 26.0), c.width(), text, regular(T_BODY), INK2);
            return;
        };
        let (side, offset) = self.place_shown(&peer.id, (peer.side, status.offset_of(&peer.id)));
        let text = match offset {
            Some((x, y)) => format!("Its top-left corner sits at {x}, {y} of your screens."),
            None if peer.screens.is_empty() => "Its monitors are known once it is connected; then drag it beside one of yours.".to_owned(),
            None => format!("Anywhere along your {} edge. Drag it next to a monitor to place it exactly.", edge_word(side)),
        };
        let h = para(ui, c.left_top() + vec2(0.0, 26.0), c.width(), &text, regular(T_BODY), INK2);
        a11y_text(ui, Id::unique(("exact-place", &peer.id)), Rect::from_min_size(c.left_top() + vec2(0.0, 26.0), vec2(c.width(), h)), Role::Label, &text, Some(Live::Polite));
        if offset.is_some() {
            let label = "Anywhere along the side";
            let b = Rect::from_min_size(pos2(c.left(), c.top() + 26.0 + h + 10.0), vec2(button_width(ui, label), 28.0));
            if button(ui, b, Id::unique(("place-anywhere", &peer.id)), label, Look::Quiet, true, Some(&format!("Let {} take the pointer anywhere along your {} edge", peer.name, edge_word(side)))).clicked() {
                self.choose_side(&peer.id, side, false);
            }
        }
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
                    let off = auto_text.is_some().then(|| place(bw("Turn off")));
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
                        && button(ui, or, Id::unique(("profile-auto-off", n)), "Turn off", Look::Quiet, true, Some(&format!("Stop switching to {n} by itself"))).clicked()
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

/// A small status dot.
pub(super) fn p_dot(ui: &Ui, at: Pos2, col: Color32) {
    ui.painter().circle_filled(at, 3.0, col);
}
