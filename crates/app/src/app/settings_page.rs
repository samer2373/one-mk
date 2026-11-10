//! Settings: eight categories as a row of pills under the title; one
//! category shows at a time, as groups of rows on cards at most 720 wide.
//! Every row is a label with its caption (and the "i" explainer) and a
//! value or control at the right; numbers, times and shortcuts in mono.
//! The palette's search lands on a row: it opens the row's category,
//! scrolls to it and lights it for a moment.

use super::pages::{GROUP_LABEL, group_card};
use super::settings_controls::{HOOK_ROW, SHORTCUT_ROW};
use super::*;

const MODS: [(&str, &str); 4] = [("none", "None"), ("shift", "Shift"), ("ctrl", "Ctrl"), ("alt", "Alt")];
const TYPING: [(&str, &str); 3] = [("physical", "Keys"), ("characters", "Letters"), ("auto", "Auto")];
const WRAP: [(&str, &str); 3] = [("off", "Off"), ("on", "Left / right"), ("all", "All edges")];
const LEARN: [(&str, &str); 3] = [("off", "Off"), ("suggest", "Ask me"), ("auto", "Place them")];
const POINTER: [(&str, &str); 3] = [("auto", "Auto"), ("always", "Always"), ("never", "Never")];
const SCOPE: [(&str, &str); 5] = [("", "Shared"), ("left", "Left"), ("right", "Right"), ("top", "Top"), ("bottom", "Bottom")];

/// A row on a page (the handoff's 52).
const ROW: f32 = 52.0;
/// The widest a category's cards go.
const PAGE_MAX: f32 = 720.0;
/// A category pill: 32 tall, a capsule.
const PILL_H: f32 = 32.0;
const PILL_GAP: f32 = 8.0;
/// Between one group and the next.
const GROUP_GAP: f32 = 18.0;
/// How long the row the palette found stays lit.
const LIT_SECS: f64 = 2.2;

/// A category of Settings, in the order of the pills.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(super) enum Cat {
    General,
    Edges,
    Power,
    Games,
    Shortcuts,
    Commands,
    Typing,
    Pointer,
}

impl Cat {
    pub(super) const ALL: [Cat; 8] = [Cat::General, Cat::Edges, Cat::Power, Cat::Games, Cat::Shortcuts, Cat::Commands, Cat::Typing, Cat::Pointer];

    pub(super) fn label(self) -> &'static str {
        match self {
            Cat::General => "General",
            Cat::Edges => "Crossing edges",
            Cat::Power => "Power & locking",
            Cat::Games => "Games",
            Cat::Shortcuts => "Shortcuts",
            Cat::Commands => "Run commands",
            Cat::Typing => "Typing",
            Cat::Pointer => "Pointer",
        }
    }

    pub(super) fn slug(self) -> &'static str {
        match self {
            Cat::General => "general",
            Cat::Edges => "edges",
            Cat::Power => "power",
            Cat::Games => "games",
            Cat::Shortcuts => "shortcuts",
            Cat::Commands => "commands",
            Cat::Typing => "typing",
            Cat::Pointer => "pointer",
        }
    }

    pub(super) fn from_slug(slug: &str) -> Option<Cat> {
        Cat::ALL.into_iter().find(|c| c.slug() == slug)
    }
}

/// A card of rows with its label above.
pub(super) struct Group {
    pub title: &'static str,
    /// The rows' keys, top to bottom. A key ending in `-note` is a line of
    /// small print under the rows above it, not a row.
    pub rows: &'static [&'static str],
}

/// What each category holds.
pub(super) fn groups(cat: Cat) -> &'static [Group] {
    match cat {
        Cat::General => &[
            Group { title: "General", rows: &["appearance", "autostart", "service"] },
            Group { title: "Updates", rows: &["update-check", "update-auto", "update-now", "update-more"] },
        ],
        Cat::Edges => &[Group { title: "Crossing edges", rows: &["edge-scope", "edge-corner", "edge-dwell", "edge-mod", "edge-fullscreen", "edge-reset", "edge-learn"] }],
        Cat::Power => &[Group { title: "Power and locking", rows: &["block-saver", "sleep-sync", "lock-sync"] }],
        Cat::Games => &[Group { title: "Games", rows: &["gaming"] }],
        Cat::Shortcuts => &[Group { title: "Keyboard shortcuts", rows: &["hotkeys", "hotkey-0", "hotkey-1", "hotkey-2", "hotkey-3", "hotkey-4", "hotkey-5", "hotkeys-note"] }],
        Cat::Commands => &[Group { title: "Run commands", rows: &["hook-0", "hook-1", "hook-2", "hook-3", "hooks-note"] }],
        Cat::Typing => &[
            Group { title: "Typing on the other computer", rows: &["typing", "typing-note"] },
            Group { title: "Ctrl and Cmd", rows: &["swaps-note", "swaps"] },
            Group { title: "Typing on every computer", rows: &["broadcast-idle"] },
        ],
        Cat::Pointer => &[Group { title: "Pointer", rows: &["pointer-wrap", "pointer-overlay"] }],
    }
}

/// What the palette finds in Settings: its title there, and where it is.
pub(super) const JUMPS: &[(&str, Cat, &str)] = &[
    ("Updates · Look for updates", Cat::General, "update-check"),
    ("Updates · Install them without asking", Cat::General, "update-auto"),
    ("Crossing edges · Which edges cross", Cat::Edges, "edge-scope"),
    ("Crossing edges · Don't cross near corners", Cat::Edges, "edge-corner"),
    ("Crossing edges · Wait at the edge", Cat::Edges, "edge-dwell"),
    ("Crossing edges · Hold a key to cross", Cat::Edges, "edge-mod"),
    ("Crossing edges · Stay in fullscreen windows", Cat::Edges, "edge-fullscreen"),
    ("Learn computer positions", Cat::Edges, "edge-learn"),
    ("Power · Keep computers I control awake", Cat::Power, "block-saver"),
    ("Power · Sleep displays together", Cat::Power, "sleep-sync"),
    ("Power · Lock every screen with this one", Cat::Power, "lock-sync"),
    ("Games · Gaming mode", Cat::Games, "gaming"),
    ("Keyboard shortcuts", Cat::Shortcuts, "hotkeys"),
    ("Shortcuts · Jump to computer N", Cat::Shortcuts, "hotkey-0"),
    ("Shortcuts · The next computer", Cat::Shortcuts, "hotkey-1"),
    ("Shortcuts · Lock the pointer here", Cat::Shortcuts, "hotkey-2"),
    ("Shortcuts · Back to this computer", Cat::Shortcuts, "hotkey-3"),
    ("Shortcuts · Type on every computer", Cat::Shortcuts, "hotkey-4"),
    ("Shortcuts · Get the next computer's clipboard", Cat::Shortcuts, "hotkey-5"),
    ("Ctrl and Cmd · Swap them between a Mac and a PC", Cat::Typing, "swaps"),
    ("Run commands when the pointer arrives or leaves", Cat::Commands, "hook-0"),
    ("Run commands · Pointer leaves", Cat::Commands, "hook-1"),
    ("Run commands · A computer connects", Cat::Commands, "hook-2"),
    ("Run commands · A computer disconnects", Cat::Commands, "hook-3"),
    ("Typing on the other computer · Letters across layouts", Cat::Typing, "typing"),
    ("Typing on every computer · Stop when idle", Cat::Typing, "broadcast-idle"),
    ("Pointer wrap", Cat::Pointer, "pointer-wrap"),
    ("Draw a pointer on computers with no mouse", Cat::Pointer, "pointer-overlay"),
];

/// The row a palette title stands for.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn jump_for(title: &str) -> Option<(Cat, &'static str)> {
    JUMPS.iter().find(|(t, _, _)| *t == title).map(|(_, cat, key)| (*cat, *key))
}

/// The category a row is in.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn cat_of(key: &str) -> Option<Cat> {
    Cat::ALL.into_iter().find(|cat| groups(*cat).iter().any(|g| g.rows.contains(&key)))
}

/// The row the palette took the person to, lit for a moment.
#[derive(Clone, Copy, Debug)]
pub(super) struct Jump {
    pub key: &'static str,
    pub at: f64,
    pub scrolled: bool,
}

fn is_note(key: &str) -> bool {
    key.ends_with("-note")
}

fn para_h(ui: &Ui, text: &str, font: egui::FontId, w: f32) -> f32 {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font.clone(), pal().ink2, w.max(1.0));
    job.sections[0].format.line_height = Some((font.size * LEADING).round());
    ui.painter().layout_job(job).size().y
}

impl App {
    /// Opens the row the palette found: its category, then the row lit.
    pub(super) fn jump_to_setting(&mut self, cat: Cat, key: &'static str) {
        self.set_tab(Tab::Settings);
        self.settings_cat = cat;
        self.settings_jump = Some(Jump { key, at: self.now, scrolled: false });
    }

    pub(super) fn settings(&mut self, ui: &mut Ui, rect: Rect) {
        let x0 = rect.left() + 4.0;
        let full_w = rect.width() - 8.0;
        let cat = self.settings_cat;
        // A shortcut being changed stops listening when its page goes.
        if cat != Cat::Shortcuts {
            ui.ctx().data_mut(|data| data.remove::<usize>(Id::unique("shortcut-recorder")));
        }
        let pills_h = self.category_pills(ui, pos2(x0, rect.top() + 6.0), full_w);
        let area = Rect::from_min_max(pos2(rect.left(), rect.top() + 6.0 + pills_h + 14.0), pos2(rect.right(), rect.bottom() - 8.0));
        let col_w = (full_w - 14.0).clamp(240.0, PAGE_MAX);
        let inner_w = col_w - 36.0;
        // Every row's height, as it is now: a row that has nothing to show is 0.
        let heights: Vec<Vec<f32>> = groups(cat).iter().map(|g| g.rows.iter().map(|k| self.setting_h(ui, k, inner_w)).collect()).collect();
        let rows_h = |hs: &Vec<f32>| hs.iter().sum::<f32>();
        let credit_h = 40.0;
        let total = groups(cat).iter().zip(&heights).map(|(_, hs)| GROUP_LABEL + rows_h(hs) + GROUP_GAP).sum::<f32>() + credit_h;

        let mut child = ui.new_child(UiBuilder::new().max_rect(area));
        child.set_clip_rect(area.expand2(vec2(0.0, 2.0)));
        let scroll = egui::ScrollArea::vertical().id_salt(("settings", cat.slug())).auto_shrink([false, false]);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        let scroll = std::env::var("ONE_MK_APP_SCROLL").ok().and_then(|value| value.parse::<f32>().ok()).map_or(scroll.clone(), |offset| scroll.vertical_scroll_offset(offset));
        scroll.show(&mut child, |ui| {
            let (all, _) = ui.allocate_exact_size(vec2(area.width(), total), Sense::hover());
            let mut y = all.top();
            for (group, hs) in groups(cat).iter().zip(&heights) {
                let rows = group_card(ui, pos2(x0, y), col_w, group.title, rows_h(hs));
                let mut top = rows.top();
                let mut first = true;
                for (key, h) in group.rows.iter().zip(hs) {
                    if *h <= 0.0 {
                        continue;
                    }
                    let r = Rect::from_min_size(pos2(rows.left(), top), vec2(rows.width(), *h));
                    if !first && !is_note(key) {
                        widgets::hairline(ui.painter(), r.x_range(), r.top());
                    }
                    first = false;
                    self.light_row(ui, key, r, rows);
                    self.setting_row(ui, key, r);
                    top += h;
                }
                y += GROUP_LABEL + rows_h(hs) + GROUP_GAP;
            }
            if cat == Cat::Shortcuts {
                self.shortcut_record(ui);
            }
            // Credits for what the window draws and isn't ours: last on the
            // page, and last in Tab order.
            let credit = "The 3D mouse is “Logitech G502 x lightspeed” by Okopchi, CC BY 4.0, changed for oneMK.";
            let credit_at = pos2(x0 + 2.0, all.bottom() - 18.0);
            let shown = fit(ui.painter(), credit, &regular(T_META), col_w - 4.0);
            let link = ui.interact(widgets::hit_rect(Rect::from_min_size(credit_at - vec2(0.0, 9.0), vec2(text_width(ui, &shown, regular(T_META)), 18.0))), Id::unique("credit-mouse"), Sense::click());
            let lr = link.rect;
            ui.painter().text(credit_at, Align2::LEFT_CENTER, &shown, regular(T_META), if link.hovered() { pal().ink } else { pal().ink3 });
            if link.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            widgets::focus_ring(ui, &link, lr, 6);
            widgets::a11y(&link, Role::Link, "Credit: open the 3D mouse model's page", None);
            if link.on_hover_text("Open the model's page").clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab("https://sketchfab.com/3d-models/logitech-g502-x-lightspeed-cda7107cc1444b4788d747f0361f7d40"));
            }
        });
    }

    /// The row the palette found: scrolled into view once, and lit on the
    /// card for a moment (a still tint with reduced motion).
    fn light_row(&mut self, ui: &Ui, key: &str, r: Rect, rows: Rect) {
        let Some(jump) = self.settings_jump.as_mut().filter(|j| j.key == key) else { return };
        let age = self.now - jump.at;
        if !jump.scrolled {
            jump.scrolled = true;
            ui.scroll_to_rect(r.expand2(vec2(0.0, 36.0)), Some(egui::Align::Center));
        }
        let left = (LIT_SECS - age).max(0.0);
        if left <= 0.0 {
            self.settings_jump = None;
            return;
        }
        let k = if motion::reduced() { 1.0 } else { (left / 0.6).min(1.0) as f32 };
        let tint = Rect::from_min_max(pos2(rows.left() - 10.0, r.top() + 2.0), pos2(rows.right() + 10.0, r.bottom() - 2.0));
        ui.painter().rect_filled(tint, CornerRadius::same(10), pal().accent_soft.gamma_multiply(k));
        ui.ctx().request_repaint_after_secs(1.0 / 30.0);
    }

    /// The row of category pills; returns its height.
    fn category_pills(&mut self, ui: &mut Ui, at: Pos2, w: f32) -> f32 {
        let (mut x, mut y) = (at.x, at.y);
        let mut chosen = None;
        // 14 either side of the word and 8 between; a little less when that
        // is the difference between one row and two (a narrow window).
        let words: f32 = Cat::ALL.iter().map(|c| text_width(ui, c.label(), semibold(T_BODY))).sum();
        let n = Cat::ALL.len() as f32;
        let (pad, gap) = if words + n * 28.0 + (n - 1.0) * PILL_GAP <= w { (28.0, PILL_GAP) } else { (24.0, 6.0) };
        for cat in Cat::ALL {
            let width = text_width(ui, cat.label(), semibold(T_BODY)) + pad;
            if x > at.x && x + width > at.x + w {
                x = at.x;
                y += PILL_H + PILL_GAP;
            }
            let r = Rect::from_min_size(pos2(x, y), vec2(width, PILL_H));
            x += width + gap;
            let id = Id::unique(("settings-cat", cat.slug()));
            let resp = ui.interact(widgets::hit_rect(r), id, widgets::sense(ui, Sense::click()));
            let on = cat == self.settings_cat;
            let hover = tween(ui.ctx(), id.with("hover"), f32::from(resp.hovered()), 0.1, motion::out_cubic);
            let lit = tween(ui.ctx(), id.with("on"), f32::from(on), motion::fade(0.15), motion::linear);
            let press = tween(ui.ctx(), id.with("press"), f32::from(resp.is_pointer_button_down_on()), motion::fade(0.06), motion::out_cubic);
            // Chosen: `ink` with `bg` text; the others `surface2` with `ink2`.
            let fill = widgets::pressed(mix(mix(pal().surface2, pal().surface2_hover, hover), pal().ink, lit), press * (1.0 - lit));
            let p = ui.painter();
            p.rect_filled(r, CornerRadius::same((PILL_H / 2.0) as u8), fill);
            let font = if on { semibold(T_BODY) } else { medium(T_BODY) };
            p.text(r.center(), Align2::CENTER_CENTER, cat.label(), font, mix(mix(pal().ink2, pal().ink_hover, hover), pal().bg, lit));
            widgets::focus_ring(ui, &resp, r, (PILL_H / 2.0) as u8);
            if resp.hovered() && !on {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            widgets::a11y(&resp, Role::Tab, cat.label(), Some(on));
            #[cfg(any(debug_assertions, feature = "design-evidence"))]
            widgets::targets::note(&format!("Settings category {}", cat.label()), r);
            if resp.clicked() && !on {
                chosen = Some(cat);
            }
        }
        if let Some(cat) = chosen {
            self.settings_cat = cat;
            self.settings_jump = None;
        }
        y + PILL_H - at.y
    }

    /// The side the edge rules show: shared, or one edge's own.
    fn edge_side(&self, ui: &Ui) -> String {
        ui.ctx().data(|data| data.get_temp::<String>(Id::unique("edge-scope"))).unwrap_or_default()
    }

    /// (corner px, wait ms, key to hold, stay in fullscreen) for `side`.
    fn edge_rules(&self, side: &str) -> (u32, u32, String, bool) {
        match self.prefs.edge_sides.iter().find(|(name, _)| name == side) {
            Some((_, rules)) => (rules.corner, rules.dwell, rules.modifier.clone(), rules.fullscreen),
            None => (self.prefs.edge_corner, self.prefs.edge_dwell_ms, self.prefs.edge_modifier.clone(), self.prefs.edge_fullscreen),
        }
    }

    /// Room under "Stay in fullscreen windows" for what this desktop can't do.
    fn fullscreen_note_h(&self, ui: &Ui, width: f32) -> f32 {
        crate::desktop::Session::current().fullscreen_note().map_or(0.0, |note| para_h(ui, note, regular(T_META), width) + 8.0)
    }

    /// A row's height now (0: nothing to show).
    fn setting_h(&self, ui: &Ui, key: &str, w: f32) -> f32 {
        let meta = |text: &str| para_h(ui, text, regular(T_META), w);
        match key {
            "update-auto" | "update-now" if !self.update_known() => 0.0,
            "update-more" => self.update_more_h(ui, w),
            "edge-fullscreen" => ROW + self.fullscreen_note_h(ui, w),
            "edge-reset" => if self.edge_side(ui).is_empty() { 0.0 } else { 48.0 },
            "hotkeys-note" => {
                let (always, issue) = self.shortcut_notes();
                meta(&always) + issue.map_or(0.0, |text| meta(&text) + 8.0) + 22.0
            }
            "hooks-note" => meta("Commands run here with your permissions. Arrivals and departures refer to this computer. Only add commands you trust.") + 22.0,
            "typing-note" => meta(&format!("{} It changes the next time the pointer crosses.", self.typing_words())) + 20.0,
            "swaps-note" => meta("Auto swaps them only between a Mac and a PC.") + 20.0,
            "swaps" => {
                let peers = self.running().map_or(0, |s| s.peers.len());
                if peers == 0 { para_h(ui, "Pair a computer to choose how Ctrl and Cmd work when you type on it.", regular(T_BODY), w) + 24.0 } else { super::swap_row_h(w) * peers as f32 + 16.0 }
            }
            k if k.starts_with("hotkey-") => k["hotkey-".len()..].parse::<usize>().ok().filter(|i| *i < self.prefs.hotkeys.len()).map_or(0.0, |_| SHORTCUT_ROW),
            k if k.starts_with("hook-") && k != "hook-shell" => HOOK_ROW,
            _ => ROW,
        }
    }

    /// What the typing preference does, in the chosen mode.
    fn typing_words(&self) -> &'static str {
        match self.prefs.typing {
            "characters" => "Experimental: the other computer types the letters printed on your keys, even with a different layout. Only Windows and Mac computers can, untested on real hardware; Linux uses the keys.",
            "auto" => "Experimental: letters when the two keyboard layouts differ, the same keys when they match. Only Windows and Mac computers can type letters; Linux uses the keys.",
            _ => "Each key you press is the same key on the other computer, whatever its layout. Letters and Auto are experimental.",
        }
    }

    /// Draws row `key` in `r`.
    fn setting_row(&mut self, ui: &mut Ui, key: &str, r: Rect) {
        match key {
            "appearance" => self.appearance_row(ui, r),
            "autostart" => {
                let on = self.autostart.unwrap_or(false);
                let busy = self.busy.iter().any(|j| matches!(j, Job::Autostart(_)));
                let resp = switch_help(ui, r, Id::unique("settings-autostart"), "Start at login", "autostart", on, self.autostart.is_some() && !busy);
                if self.autostart.is_none() {
                    resp.on_hover_text("Checking…");
                } else if resp.clicked() {
                    // Shown at once; the link reports what really happened.
                    self.busy.push(Job::Autostart(!on));
                    self.autostart = Some(!on);
                    self.link.set_autostart(!on);
                }
            }
            "service" => self.service_row(ui, r),
            "update-check" | "update-auto" | "update-now" | "update-more" => self.update_row(ui, key, r),
            "edge-scope" => {
                let mut side = self.edge_side(ui);
                if let Some(k) = self.seg_row(ui, r, "edge-scope", "Which edges cross", "edge-scope", &SCOPE, &side, None) {
                    side = SCOPE[k].0.to_owned();
                    ui.ctx().data_mut(|data| data.insert_temp(Id::unique("edge-scope"), side));
                }
            }
            "edge-corner" | "edge-dwell" => self.edge_number_row(ui, key, r),
            "edge-mod" => {
                let side = self.edge_side(ui);
                let (_, _, modifier, _) = self.edge_rules(&side);
                if let Some(k) = self.seg_row(ui, r, "edge-mod", "Hold a key to cross", "edge-mod", &MODS, &modifier, None) {
                    self.edge_setting(&side, "--modifier", MODS[k].0);
                }
            }
            "edge-fullscreen" => {
                let side = self.edge_side(ui);
                let (_, _, _, fullscreen) = self.edge_rules(&side);
                let row = Rect::from_min_size(r.min, vec2(r.width(), ROW));
                if switch_help(ui, row, Id::unique("edge-fullscreen"), "Stay in fullscreen windows", "edge-fullscreen", fullscreen, true).clicked() {
                    self.edge_setting(&side, "--fullscreen-guard", if fullscreen { "off" } else { "on" });
                }
                // Where this desktop can't see every fullscreen window, say so under the switch.
                if let Some(note) = crate::desktop::Session::current().fullscreen_note() {
                    para(ui, pos2(r.left(), r.top() + ROW - 8.0), r.width(), note, regular(T_META), pal().ink3);
                }
            }
            "edge-reset" => {
                let side = self.edge_side(ui);
                let b = Rect::from_min_size(pos2(r.left(), r.center().y - 14.0), vec2(button_width(ui, "Use shared rules"), 28.0));
                if button(ui, b, Id::unique("edge-reset-side"), "Use shared rules", Look::Quiet, true, None).clicked() {
                    self.settings_command("the edge rules", vec!["edge".into(), "--side".into(), side, "--reset".into()]);
                }
            }
            "edge-learn" => {
                let current = self.prefs.edge_learn.clone();
                if let Some(k) = self.seg_row(ui, r, "edge-learn", "Learn computer positions", "edge-learn", &LEARN, &current, None) {
                    self.settings_command("Learn computer positions", vec!["edge".into(), "--learn".into(), LEARN[k].0.into()]);
                }
            }
            "pointer-wrap" => {
                let current = self.prefs.edge_wrap.clone();
                if let Some(k) = self.seg_row(ui, r, "pointer-wrap", "Pointer wrap", "edge-wrap", &WRAP, &current, None) {
                    self.settings_command("Pointer wrap", vec!["edge".into(), "--wrap".into(), WRAP[k].0.into()]);
                }
            }
            "pointer-overlay" => {
                let current = self.prefs.pointer_draw.clone();
                if let Some(k) = self.seg_row(ui, r, "pointer-overlay", "Draw a pointer on computers with no mouse", "pointer-overlay", &POINTER, &current, None) {
                    self.settings_command("Draw a pointer on computers with no mouse", vec!["pointer-overlay".into(), POINTER[k].0.into()]);
                }
            }
            "block-saver" => {
                let on = self.prefs.block_screensaver;
                if switch_help(ui, r, Id::unique("block-saver"), "Keep computers I control awake", "block-saver", on, true).clicked() {
                    self.set_pref("keeping computers awake", &["power", "--block-screensaver", if on { "off" } else { "on" }], |p| p.block_screensaver = !p.block_screensaver);
                }
            }
            "sleep-sync" => {
                let on = self.prefs.sleep_sync;
                if switch_help(ui, r, Id::unique("sleep-sync"), "Sleep displays together", "sleep-sync", on, true).clicked() {
                    self.set_pref("sleep sync", &["power", "--sleep-sync", if on { "off" } else { "on" }], |p| p.sleep_sync = !p.sleep_sync);
                }
            }
            "lock-sync" => {
                let on = self.prefs.lock_sync;
                if switch_help(ui, r, Id::unique("lock-sync"), "Lock every screen with this one", "lock-sync", on, true).clicked() {
                    self.set_pref("lock sync", &["lock-sync", if on { "off" } else { "on" }], |p| p.lock_sync = !p.lock_sync);
                }
            }
            "gaming" => self.gaming_row(ui, r),
            "hotkeys" => {
                let on = self.prefs.hotkeys_on;
                if switch_help(ui, r, Id::unique("hotkeys"), "Use keyboard shortcuts", "hotkeys", on, true).clicked() {
                    self.set_pref("keyboard shortcuts", &["hotkeys", if on { "--off" } else { "--on" }], |prefs| prefs.hotkeys_on = !prefs.hotkeys_on);
                }
            }
            "hotkeys-note" => {
                let (always, issue) = self.shortcut_notes();
                let mut y = r.top() + 6.0;
                y += para(ui, pos2(r.left(), y), r.width(), &always, regular(T_META), pal().ink2) + 8.0;
                if let Some(text) = issue {
                    para(ui, pos2(r.left(), y), r.width(), &text, regular(T_META), pal().ink2);
                }
            }
            "hooks-note" => {
                para(ui, pos2(r.left(), r.top() + 6.0), r.width(), "Commands run here with your permissions. Arrivals and departures refer to this computer. Only add commands you trust.", regular(T_META), pal().ink2);
            }
            "typing" => {
                let labels: Vec<&str> = TYPING.iter().map(|(_, l)| *l).collect();
                let current = TYPING.iter().find(|(v, _)| *v == self.prefs.typing).map_or("", |(v, _)| *v);
                let names: Vec<String> = TYPING.iter().map(|(value, label)| {
                    if *value == "physical" { format!("Typing preference: {label}") } else { format!("Typing preference: {label} (experimental)") }
                }).collect();
                let _ = labels;
                if let Some(k) = self.seg_row(ui, r, "typing", "Typing preference", "typing", &TYPING, current, Some(names)) {
                    let mode = TYPING[k].0;
                    self.set_pref("typing preference", &["typing", "--keys", mode], |prefs| prefs.typing = TYPING.iter().find(|(v, _)| *v == mode).map_or("physical", |(v, _)| *v));
                }
            }
            "typing-note" => {
                para(ui, pos2(r.left(), r.top() + 2.0), r.width(), &format!("{} It changes the next time the pointer crosses.", self.typing_words()), regular(T_META), pal().ink2);
            }
            "swaps-note" => {
                para(ui, pos2(r.left(), r.top() + 10.0), r.width(), "Auto swaps them only between a Mac and a PC.", regular(T_META), pal().ink2);
            }
            "swaps" => self.swaps(ui, Rect::from_min_max(r.min + vec2(0.0, 8.0), r.max - vec2(0.0, 8.0))),
            "broadcast-idle" => self.broadcast_timeout(ui, r),
            k if k.starts_with("hotkey-") => {
                if let Ok(index) = k["hotkey-".len()..].parse::<usize>() {
                    self.shortcut_row(ui, index, r);
                }
            }
            k if k.starts_with("hook-") => {
                if let Ok(index) = k["hook-".len()..].parse::<usize>() {
                    self.hook_row(ui, index, r);
                }
            }
            _ => {}
        }
    }

    /// A row of a label (with its caption and "i") and a segmented control
    /// at the right; returns the choice made, by place in `choices`.
    #[allow(clippy::too_many_arguments)]
    fn seg_row(&mut self, ui: &mut Ui, r: Rect, id: &str, label: &str, help: &str, choices: &[(&str, &str)], current: &str, names: Option<Vec<String>>) -> Option<usize> {
        let labels: Vec<&str> = choices.iter().map(|(_, l)| *l).collect();
        // Cells are equal: as wide as the longest word (chosen, in 600) needs.
        let cell = labels.iter().map(|l| text_width(ui, l, semibold(T_META))).fold(0.0, f32::max) + 24.0;
        let seg_w = (cell * labels.len() as f32 + 2.0 * labels.len() as f32 + 4.0).max(150.0).min(r.width() * 0.6);
        label_help(ui, r, Id::unique(("seg-label", id)), label, help, true, seg_w + 12.0);
        let track = Rect::from_min_size(pos2(r.right() - seg_w, r.center().y - 14.0), vec2(seg_w, 28.0));
        Pills {
            id: Id::unique(("seg-row", id)),
            track,
            cells: Pills::even(track, choices.len(), 2.0),
            labels: &labels,
            names: names.unwrap_or_else(|| labels.iter().map(|l| format!("{label}: {l}")).collect()),
            chosen: choices.iter().position(|(value, _)| *value == current),
            enabled: vec![true; choices.len()],
            role: Role::RadioButton,
            font: T_META,
        }
        .show(ui, self.arrowed)
    }

    /// "Don't cross near corners" (px) and "Wait at the edge" (ms): a typed
    /// number with its unit, in mono.
    fn edge_number_row(&mut self, ui: &mut Ui, key: &str, r: Rect) {
        let side = self.edge_side(ui);
        let (corner, dwell, _, _) = self.edge_rules(&side);
        let (label, unit, cur, max, flag) = if key == "edge-corner" { ("Don't cross near corners", "px", corner, 500u32, "--corner") } else { ("Wait at the edge", "ms", dwell, 5000, "--dwell") };
        let uw = text_width(ui, "px", mono(T_META)).max(text_width(ui, "ms", mono(T_META))) + 8.0;
        label_help(ui, r, Id::unique(("edge-label", key)), label, key, true, 72.0 + uw + 12.0);
        let fr = Rect::from_min_size(pos2(r.right() - 72.0 - uw, r.center().y - 14.0), vec2(72.0, 28.0));
        ui.painter().text(pos2(r.right(), r.center().y), Align2::RIGHT_CENTER, unit, mono(T_META), pal().ink2);
        let a11y = format!("{label}, in {}", if unit == "px" { "pixels (0: off)" } else { "milliseconds (0: off)" });
        if let Some(text) = self.edit_field(ui, fr, &format!("edge-{}", &key["edge-".len()..]), &cur.to_string(), "0", &a11y, true, true) {
            match text.parse::<u32>() {
                Ok(n) if n <= max => self.edge_setting(&side, flag, &n.to_string()),
                _ => self.toast_error(format!("{label}: type a whole number from 0 to {max}."), None, self.now),
            }
        }
    }

    /// Whether oneMK runs here (as the status line says it) and the way to
    /// stop it, or to start it again.
    fn service_row(&mut self, ui: &mut Ui, row: Rect) {
        let (tone, text) = self.status_line();
        let running = self.running().is_some();
        let busy = self.busy.contains(&Job::Stop) || self.busy.contains(&Job::Start);
        let (label, name) = match (running, busy) {
            (true, true) => ("Stopping…", "Stopping oneMK"),
            (true, false) => ("Stop", "Stop oneMK: turn off sharing on this computer"),
            (false, true) => ("Starting…", "Starting oneMK"),
            (false, false) => ("Start oneMK", "Start oneMK"),
        };
        let bw = button_width(ui, label);
        widgets::status_mark(ui.painter(), pos2(row.left() + 3.5, row.center().y), 7.0, widgets::Mark::of(tone));
        let shown = fit(ui.painter(), &text, &regular(T_BODY), row.width() - bw - 30.0);
        ui.painter().text(pos2(row.left() + 16.0, row.center().y), Align2::LEFT_CENTER, shown, regular(T_BODY), pal().ink);
        a11y_text(ui, Id::unique("service-state"), Rect::from_min_max(row.min, pos2(row.right() - bw - 12.0, row.bottom())), Role::Label, &text, None);
        let b = Rect::from_min_size(pos2(row.right() - bw, row.center().y - 16.0), vec2(bw, 32.0));
        let can = self.status.is_some() && !busy && !self.sheet_open();
        if button(ui, b, Id::unique("stop"), label, Look::Quiet, can, Some(name)).clicked() {
            if running {
                self.busy.push(Job::Stop);
                self.link.stop();
            } else {
                self.busy.push(Job::Start);
                self.link.start();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn eight_categories_in_the_order_of_the_handoff() {
        let labels: Vec<&str> = Cat::ALL.iter().map(|c| c.label()).collect();
        assert_eq!(labels, ["General", "Crossing edges", "Power & locking", "Games", "Shortcuts", "Run commands", "Typing", "Pointer"]);
        for cat in Cat::ALL {
            assert_eq!(Cat::from_slug(cat.slug()), Some(cat));
            assert!(!groups(cat).is_empty() && groups(cat).iter().all(|g| !g.rows.is_empty()), "{cat:?} has rows");
        }
    }

    #[test]
    fn every_row_is_in_one_category_only() {
        let mut seen = HashSet::new();
        for cat in Cat::ALL {
            for group in groups(cat) {
                for key in group.rows {
                    assert!(seen.insert(*key), "{key} is on two pages");
                    assert_eq!(cat_of(key), Some(cat));
                }
            }
        }
    }

    #[test]
    fn each_old_section_lives_in_a_sensible_category() {
        let on = |key: &str| cat_of(key).unwrap();
        // Updates and Appearance are in General, with Start at login and Stop.
        for key in ["appearance", "autostart", "service", "update-check", "update-auto", "update-now"] {
            assert_eq!(on(key), Cat::General, "{key}");
        }
        for key in ["edge-corner", "edge-dwell", "edge-mod", "edge-fullscreen", "edge-learn"] {
            assert_eq!(on(key), Cat::Edges, "{key}");
        }
        for key in ["block-saver", "sleep-sync", "lock-sync"] {
            assert_eq!(on(key), Cat::Power, "{key}");
        }
        assert_eq!(on("gaming"), Cat::Games);
        assert_eq!(on("hotkeys"), Cat::Shortcuts);
        assert_eq!(on("hotkey-5"), Cat::Shortcuts);
        for key in ["hook-0", "hook-3", "hooks-note"] {
            assert_eq!(on(key), Cat::Commands, "{key}");
        }
        for key in ["typing", "swaps", "broadcast-idle"] {
            assert_eq!(on(key), Cat::Typing, "{key}");
        }
        for key in ["pointer-wrap", "pointer-overlay"] {
            assert_eq!(on(key), Cat::Pointer, "{key}");
        }
    }

    #[test]
    fn the_hooks_and_shortcuts_the_old_page_had_are_all_here() {
        let rows: Vec<&str> = Cat::ALL.iter().flat_map(|c| groups(*c).iter().flat_map(|g| g.rows.iter().copied())).collect();
        for k in 0..crate::view::HOOK_EVENTS.len() {
            assert!(rows.contains(&format!("hook-{k}").as_str()));
        }
        for (k, shortcut) in one_mk_proto::shortcuts::defaults(false).iter().enumerate() {
            assert!(rows.contains(&format!("hotkey-{k}").as_str()), "shortcut {k}");
            // Each can be found by name in the palette.
            let found = JUMPS.iter().find(|j| j.2 == format!("hotkey-{k}"));
            assert!(found.is_some_and(|j| j.0.ends_with(shortcut.label)), "{}", shortcut.label);
        }
        for (k, (_, label)) in crate::view::HOOK_EVENTS.iter().enumerate().skip(1) {
            let found = JUMPS.iter().find(|j| j.2 == format!("hook-{k}"));
            assert!(found.is_some_and(|j| j.0.ends_with(label)), "{label}");
        }
    }

    #[test]
    fn the_palette_finds_every_setting_and_lands_on_its_row() {
        // Every Settings entry the palette lists points at a real row, in
        // the category that row is on.
        for (title, cat, key) in JUMPS {
            assert_eq!(jump_for(title), Some((*cat, *key)));
            assert_eq!(cat_of(key), Some(*cat), "{title}");
        }
        // Titles are unique (the first match would hide the others).
        let titles: HashSet<_> = JUMPS.iter().map(|j| j.0).collect();
        assert_eq!(titles.len(), JUMPS.len());
        // Every settings row that is a control (not small print or a
        // follow-on) is reachable by a title, or sits under one that is.
        for key in ["update-check", "update-auto", "edge-scope", "edge-corner", "edge-dwell", "edge-mod", "edge-fullscreen", "edge-learn", "block-saver", "sleep-sync", "lock-sync", "gaming", "hotkeys", "swaps", "hook-0", "typing", "broadcast-idle", "pointer-wrap", "pointer-overlay"] {
            assert!(JUMPS.iter().any(|j| j.2 == key), "no palette entry for {key}");
        }
        assert_eq!(jump_for("Pointer wrap"), Some((Cat::Pointer, "pointer-wrap")));
        assert_eq!(jump_for("Crossing edges · Wait at the edge"), Some((Cat::Edges, "edge-dwell")));
        assert_eq!(jump_for("Not a setting"), None);
    }
}
