//! Two full pages without the desk: Health (`one-mk doctor`, run on open
//! and on Recheck, off the window's thread, plus what the daemon says
//! right now) and Advanced (edge rules, power and locking, gaming mode,
//! keyboard shortcuts, Ctrl and Cmd, commands to run).

use super::*;
use crate::view::{self, Check, Verdict};

/// Doctor results older than this are run again when Health opens.
const STALE: f64 = 30.0;
/// A row of the Crossing edges card.
const EDGE_ROW: f32 = 38.0;
const MODS: [(&str, &str); 4] = [("none", "None"), ("shift", "Shift"), ("ctrl", "Ctrl"), ("alt", "Alt")];

/// A card: a raised panel with a title.
pub(super) fn card(ui: &Ui, r: Rect, title: &str) -> Rect {
    let p = ui.painter();
    p.rect(r, CornerRadius::same(12), PANEL, Stroke::new(1.0, LINE), StrokeKind::Inside);
    p.text(r.left_top() + vec2(18.0, 26.0), Align2::LEFT_CENTER, title, semibold(T_TITLE), INK);
    a11y_text(ui, Id::unique(("card", title)), Rect::from_min_size(r.min, vec2(r.width(), 44.0)), Role::Heading, title, None);
    Rect::from_min_max(r.min + vec2(18.0, 48.0), r.max - vec2(18.0, 14.0))
}

/// What the window can do itself about a failed check.
#[derive(Clone, Copy)]
enum Fix {
    Start,
    Pair,
}

fn verdict_icon(p: &Painter, c: Pos2, v: Verdict) {
    match v {
        Verdict::Pass => widgets::draw_check(p, c, 12.0, INK2, 1.0),
        Verdict::Skip => {
            p.line_segment([c - vec2(4.0, 0.0), c + vec2(4.0, 0.0)], Stroke::new(1.5, OFF));
        }
        Verdict::Warn => {
            // A triangle with a bar and a dot: not a ring, which reads as an
            // empty radio button.
            let (top, l, r) = (c + vec2(0.0, -6.5), c + vec2(-7.0, 5.5), c + vec2(7.0, 5.5));
            p.add(egui::Shape::closed_line(vec![top, r, l], Stroke::new(1.5, INK)));
            p.line_segment([c + vec2(0.0, -2.0), c + vec2(0.0, 1.5)], Stroke::new(1.5, INK));
            p.circle_filled(c + vec2(0.0, 3.6), 0.9, INK);
        }
        Verdict::Fail => {
            p.circle_filled(c, 6.0, ERROR);
        }
    }
}

fn verdict_word(v: Verdict) -> &'static str {
    match v {
        Verdict::Pass => "OK",
        Verdict::Skip => "Skipped",
        Verdict::Warn => "Warning",
        Verdict::Fail => "Problem",
    }
}

impl App {
    /// Runs the checks if there are none yet, or they are old.
    pub(super) fn doctor_on_open(&mut self) {
        let stale = self.doctor.as_ref().is_none_or(|(_, at)| self.now - at > STALE);
        if stale && !self.busy.contains(&Job::Doctor) {
            self.recheck();
        }
    }

    /// A check the window itself can fix: the button to show instead of
    /// copying a command.
    fn window_fix(&self, c: &Check) -> Option<(&'static str, Fix)> {
        if c.verdict == Verdict::Pass {
            return None;
        }
        match c.id.as_str() {
            "daemon" if self.off() => Some(("Start one·mk", Fix::Start)),
            // Pairing works with one·mk off too; it connects once started.
            "pairing" if self.running().is_none_or(|s| s.peers.is_empty()) => Some(("Pair a computer", Fix::Pair)),
            _ => None,
        }
    }

    fn recheck(&mut self) {
        self.busy.push(Job::Doctor);
        self.link.doctor();
    }

    /// Lines under the Health title that come from the daemon itself.
    fn live_lines(&self) -> Vec<(Color32, String)> {
        let Some(s) = self.running() else {
            return vec![(ERROR, "one·mk isn't running on this computer. Start it with the button at the top.".into())];
        };
        let mut out: Vec<(Color32, String)> = s.input.iter().map(|l| (ERROR, l.clone())).collect();
        if out.is_empty() {
            out.push((INK2, "Mouse and keyboard sharing works: the desktop allowed it.".into()));
        }
        if let Some(h) = s.hotkeys_line() {
            out.push((INK2, h));
        }
        out
    }

    pub(super) fn health(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        let w = rect.width().min(760.0) - 64.0;
        let x = rect.center().x - w / 2.0;
        let mut y = rect.top() + 20.0;
        let running = self.busy.contains(&Job::Doctor);
        let p = ui.painter().clone();
        p.text(pos2(x, y + 12.0), Align2::LEFT_CENTER, "Health", semibold(T_TITLE), INK);
        let summary = match &self.doctor {
            _ if running => "Checking…".to_owned(),
            Some((Ok(checks), _)) => view::doctor_summary(checks),
            Some((Err(e), _)) => format!("The checks didn't run: {e}"),
            None => String::new(),
        };
        let label = if running { "Checking…" } else { "Recheck" };
        let bw = button_width(ui, label);
        let b = Rect::from_min_size(pos2(x + w - bw, y - 4.0), vec2(bw, 32.0));
        if button(ui, b, Id::unique("recheck"), label, Look::Quiet, !running, Some("Run the checks again")).clicked() {
            self.recheck();
        }
        let sh = para(ui, pos2(x, y + 28.0), w - bw - 16.0, &summary, regular(T_BODY), INK2);
        a11y_text(ui, Id::unique("health-summary"), Rect::from_min_size(pos2(x, y), vec2(w, 44.0)), Role::Status, &format!("Health. {summary}"), Some(Live::Polite));
        y += 28.0 + sh + 18.0;

        // What the daemon says right now.
        let lines = self.live_lines();
        let mut h = 16.0;
        let heights: Vec<f32> = lines.iter().map(|(_, l)| {
            let g = ui.painter().layout(l.clone(), regular(T_BODY), INK, w - 50.0);
            g.size().y
        }).collect();
        h += heights.iter().map(|k| k + 8.0).sum::<f32>() + 8.0;
        let boxr = Rect::from_min_size(pos2(x, y), vec2(w, h));
        p.rect(boxr, CornerRadius::same(10), RAISED, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let mut ly = y + 12.0;
        for ((col, line), lh) in lines.iter().zip(&heights) {
            p.circle_filled(pos2(x + 20.0, ly + 9.0), 3.0, *col);
            widgets::para_code(ui, pos2(x + 34.0, ly), w - 50.0, line, regular(T_BODY), INK);
            ly += lh + 8.0;
        }
        a11y_text(ui, Id::unique("health-live"), boxr, Role::Label, &lines.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>().join(" "), None);
        y += h + 14.0;

        let list = Rect::from_min_max(pos2(x - 8.0, y), pos2(x + w + 8.0, rect.bottom() - 8.0));
        let checks = match &self.doctor {
            Some((Ok(c), _)) => c.clone(),
            _ => {
                if running {
                    // A spinner while it runs, nothing after.
                    let turn = if motion::reduced() { 0.0 } else { (now * 5.0) as f32 };
                    widgets::draw_refresh(&p, Rect::from_center_size(pos2(x + 8.0, y + 16.0), vec2(16.0, 16.0)), INK2, turn);
                    p.text(pos2(x + 24.0, y + 16.0), Align2::LEFT_CENTER, "Running the checks (a few seconds)…", regular(T_BODY), INK2);
                    ui.ctx().request_repaint_after_secs(1.0 / 30.0);
                }
                return;
            }
        };
        let copied = self.copied;
        let mut copy = None;
        let fixes: Vec<_> = checks.iter().map(|c| self.window_fix(c)).collect();
        let mut fix_now = None;
        let mut child = ui.new_child(UiBuilder::new().max_rect(list));
        child.set_clip_rect(list);
        egui::ScrollArea::vertical().id_salt("health-checks").auto_shrink([false, false]).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (k, c) in checks.iter().enumerate() {
                let inner = w - 34.0;
                let p = ui.painter().clone();
                let lay = |t: &str, f: egui::FontId, wid: f32| {
                    let mut job = egui::text::LayoutJob::simple(t.to_owned(), f.clone(), INK, wid);
                    job.sections[0].format.line_height = Some((f.size * LEADING).round());
                    p.layout_job(job).size().y
                };
                let dh = lay(&c.detail, regular(T_BODY), inner);
                let (cmd, is_cmd) = c.fix.as_deref().map(view::fix_command).unwrap_or_default();
                let here = fixes[k];
                let copy_label = if copied.as_ref().is_some_and(|(i, t)| *i == k && now - t < 1.5) {
                    "Copied"
                } else if is_cmd {
                    "Copy command"
                } else {
                    "Copy"
                };
                let cw = button_width(ui, "Copy command") - 4.0;
                // Fixed here: one button says what to do, without the command.
                let fix_text = if let Some((label, _)) = &here { Some(format!("Choose {label}.")) } else { c.fix.clone() };
                let fh = fix_text.as_ref().map_or(0.0, |f| lay(f, regular(T_BODY), inner - cw - 12.0).max(28.0) + 8.0);
                let h = 14.0 + 20.0 + dh + fh + 14.0;
                let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
                let rx = row.left() + 8.0;
                let ty = row.top() + 14.0;
                verdict_icon(&p, pos2(rx + 8.0, ty + 10.0), c.verdict);
                p.text(pos2(rx + 26.0, ty + 10.0), Align2::LEFT_CENTER, &c.title, semibold(T_BODY), INK);
                let word = verdict_word(c.verdict);
                let wc = if c.verdict == Verdict::Fail { INK } else { INK2 };
                p.text(pos2(row.right() - 8.0, ty + 10.0), Align2::RIGHT_CENTER, word, regular(T_META), wc);
                widgets::para_code(ui, pos2(rx + 26.0, ty + 22.0), inner, &c.detail, regular(T_BODY), INK2);
                if let Some(fix) = &fix_text {
                    let fy = ty + 22.0 + dh + 8.0;
                    widgets::para_code(ui, pos2(rx + 26.0, fy + 4.0), inner - cw - 12.0, fix, regular(T_BODY), INK);
                    let br = Rect::from_min_size(pos2(row.right() - 8.0 - cw, fy), vec2(cw, 28.0));
                    if let Some((label, job)) = &here {
                        if button(ui, br, Id::unique(("fix-here", k)), label, Look::Quiet, !running, None).clicked() {
                            fix_now = Some(*job);
                        }
                    } else {
                        let name = format!("Copy the fix for {}: {cmd}", c.title);
                        if button(ui, br, Id::unique(("copy-fix", k)), copy_label, Look::Quiet, true, Some(&name)).clicked() {
                            copy = Some((k, cmd.clone()));
                        }
                    }
                }
                let label = format!("{}: {}. {}{}", c.title, word, c.detail, fix_text.as_ref().map_or(String::new(), |f| format!(" Fix: {f}")));
                a11y_text(ui, Id::unique(("check", k)), row, Role::ListItem, &label, None);
                if k + 1 < checks.len() {
                    p.hline(row.x_range().shrink(8.0), row.bottom() - 0.5, Stroke::new(1.0, LINE));
                }
            }
        });
        if let Some((k, text)) = copy {
            ui.ctx().copy_text(text);
            self.copied = Some((k, now));
            ui.ctx().request_repaint_after_secs(1.6);
        }
        match fix_now {
            Some(Fix::Start) if !self.busy.contains(&Job::Start) => {
                self.busy.push(Job::Start);
                self.link.start();
            }
            Some(Fix::Pair) => self.start_pairing(now),
            _ => {}
        }
    }

    pub(super) fn advanced(&mut self, ui: &mut Ui, rect: Rect) {
        let area = Rect::from_min_max(rect.min + vec2(24.0, 16.0), rect.max - vec2(24.0, 12.0));
        let two = area.width() >= 700.0;
        let gap = 16.0;
        let col_w = if two { (area.width() - gap) / 2.0 } else { area.width() };
        // Each card as tall as what it holds: a title (48), rows, a margin (14).
        let s = self.running();
        let per_side = s.is_some_and(|s| s.edges.contains(" edge: "));
        let peers = s.map_or(0, |s| s.peers.len());
        let keys = self.prefs.hotkeys.len() + 1;
        let note = s.is_some_and(|s| view::hotkeys_note(&s.hotkeys, &s.hotkeys_detail).is_some());
        let hooks = self.prefs.hooks.len() + usize::from(self.prefs.unreadable);
        let heights = [
            48.0 + 4.0 * EDGE_ROW - 4.0 + if per_side { 22.0 } else { 0.0 } + 14.0,
            48.0 + 3.0 * 36.0 - 6.0 + 14.0,
            48.0 + 36.0 - 6.0 + 14.0,
            48.0 + 44.0 + 22.0 * keys as f32 + if note { 40.0 } else { 0.0 } + 14.0,
            48.0 + 22.0 + (SWAP_ROW * peers as f32).max(40.0) + 14.0,
            48.0 + if hooks == 0 { 44.0 } else { 22.0 * hooks as f32 + 20.0 } + 14.0,
        ];
        // Two columns: crossing, power and games on the left; keys and
        // commands on the right. One column below 700 points.
        let mut at = [(0usize, 0.0f32); 6];
        let mut ys = [0.0f32; 2];
        for (k, h) in heights.iter().enumerate() {
            let col = if two { usize::from(k >= 3) } else { 0 };
            at[k] = (col, ys[col]);
            ys[col] += h + gap;
        }
        let total = ys[0].max(ys[1]) - gap;
        let mut child = ui.new_child(UiBuilder::new().max_rect(area.expand2(vec2(6.0, 0.0))));
        child.set_clip_rect(area.expand2(vec2(8.0, 4.0)));
        egui::ScrollArea::vertical().id_salt("advanced").auto_shrink([false, false]).show(&mut child, |ui| {
            let (all, _) = ui.allocate_exact_size(vec2(area.width(), total), Sense::hover());
            let r = |k: usize| Rect::from_min_size(pos2(all.left() + at[k].0 as f32 * (col_w + gap), all.top() + at[k].1), vec2(col_w, heights[k]));
            self.edges_card(ui, r(0));
            self.power_card(ui, r(1));
            let c = card(ui, r(2), "Games");
            self.gaming_row(ui, Rect::from_min_size(pos2(c.left(), c.top() - 6.0), vec2(c.width(), 36.0)));
            self.keys_card(ui, r(3));
            let c = card(ui, r(4), "Ctrl and Cmd");
            ui.painter().text(pos2(c.left(), c.top() + 2.0), Align2::LEFT_CENTER, fit(ui.painter(), "Auto swaps them only between a Mac and a PC.", &regular(T_META), c.width()), regular(T_META), INK2);
            self.swaps(ui, Rect::from_min_max(pos2(c.left(), c.top() + 18.0), c.max));
            self.hooks_card(ui, r(5));
        });
    }

    fn edges_card(&mut self, ui: &mut Ui, r: Rect) {
        let c = card(ui, r, "Crossing edges");
        let p = self.prefs.clone();
        let top = c.top() - 4.0;
        let row = |k: usize| Rect::from_min_size(pos2(c.left(), top + EDGE_ROW * k as f32), vec2(c.width(), EDGE_ROW));
        // Numbers with a unit, typed; the fields line up whatever the unit.
        let uw = text_width(ui, "px", regular(T_META)).max(text_width(ui, "ms", regular(T_META))) + 8.0;
        for (k, (key, label, unit, cur, max, flag)) in [
            ("corner", "Don't cross near corners", "px", p.edge_corner, 500u32, "--corner"),
            ("dwell", "Wait at the edge", "ms", p.edge_dwell_ms, 5000, "--dwell"),
        ]
        .into_iter()
        .enumerate()
        {
            let rr = row(k);
            ui.painter().text(rr.left_center(), Align2::LEFT_CENTER, label, regular(T_BODY), INK);
            let fr = Rect::from_min_size(pos2(rr.right() - 72.0 - uw, rr.center().y - 13.0), vec2(72.0, 26.0));
            ui.painter().text(pos2(rr.right(), rr.center().y), Align2::RIGHT_CENTER, unit, regular(T_META), INK2);
            let a11y = format!("{label}, in {}", if unit == "px" { "pixels (0: off)" } else { "milliseconds (0: off)" });
            if let Some(text) = self.edit_field(ui, fr, &format!("edge-{key}"), &cur.to_string(), "0", &a11y, true, true) {
                match text.parse::<u32>() {
                    Ok(n) if n <= max => {
                        let v = n.to_string();
                        self.set_pref("the edge rules", &["edge", flag, &v], |p| if key == "corner" { p.edge_corner = n } else { p.edge_dwell_ms = n });
                    }
                    _ => self.toast_error(format!("{label}: type a whole number from 0 to {max}."), None, self.now),
                }
            }
            let tip = if key == "corner" { "The pointer stays here within this distance of a corner, so corners and close buttons stay easy to hit." } else { "The pointer has to rest against the edge this long before it crosses." };
            let lw = text_width(ui, label, regular(T_BODY));
            ui.interact(Rect::from_min_size(rr.min, vec2(lw, rr.height())), Id::unique(("edge-tip", key)), Sense::hover()).on_hover_text(tip);
        }
        let rr = row(2);
        let word = "Hold a key to cross";
        ui.painter().text(rr.left_center(), Align2::LEFT_CENTER, word, regular(T_BODY), INK);
        let lw = text_width(ui, word, regular(T_BODY)) + 16.0;
        let seg_w = 200.0f32.min(rr.width() - lw);
        let track = Rect::from_min_size(pos2(rr.right() - seg_w, rr.center().y - 13.0), vec2(seg_w, 26.0));
        let labels: Vec<&str> = MODS.iter().map(|(_, l)| *l).collect();
        let pills = Pills {
            id: Id::unique("edge-mod"),
            track,
            cells: Pills::even(track, 4, 2.0),
            labels: &labels,
            names: MODS.iter().map(|(_, l)| format!("Key to hold to cross: {l}")).collect(),
            chosen: MODS.iter().position(|(v, _)| *v == p.edge_modifier),
            enabled: vec![true; 4],
            role: Role::RadioButton,
            font: T_META,
            radius: 6,
        };
        if let Some(k) = pills.show(ui, self.arrowed) {
            let v = MODS[k].0;
            self.set_pref("the edge rules", &["edge", "--modifier", v], |p| p.edge_modifier = v.into());
        }
        let r = switch(ui, row(3), Id::unique("edge-fullscreen"), "Stay in fullscreen windows", p.edge_fullscreen, true);
        if r.clicked() {
            let v = if p.edge_fullscreen { "off" } else { "on" };
            self.set_pref("the edge rules", &["edge", "--fullscreen-guard", v], |p| p.edge_fullscreen = !p.edge_fullscreen);
        }
        r.on_hover_text("Don't cross while a fullscreen window (a game, a video) has the focus here.");
        let per_side = self.running().is_some_and(|s| s.edges.contains(" edge: "));
        if per_side {
            let note = "Some edges have rules of their own: see `one-mk edge`.";
            widgets::para_code(ui, pos2(c.left(), row(4).top() + 4.0), c.width(), note, regular(T_META), INK2);
        }
    }

    fn power_card(&mut self, ui: &mut Ui, r: Rect) {
        let c = card(ui, r, "Power and locking");
        let p = self.prefs.clone();
        let row_h = 36.0;
        let top = c.top() - 6.0;
        let row = |k: usize| Rect::from_min_size(pos2(c.left(), top + row_h * k as f32), vec2(c.width(), row_h));
        let r0 = switch(ui, row(0), Id::unique("block-saver"), "Keep computers I control awake", p.block_screensaver, true);
        if r0.clicked() {
            let v = if p.block_screensaver { "off" } else { "on" };
            self.set_pref("keeping computers awake", &["power", "--block-screensaver", v], |p| p.block_screensaver = !p.block_screensaver);
        }
        r0.on_hover_text("No screensaver or sleep on a computer while this one controls it.");
        if switch(ui, row(1), Id::unique("sleep-sync"), "Sleep displays together", p.sleep_sync, true).clicked() {
            let v = if p.sleep_sync { "off" } else { "on" };
            self.set_pref("sleep sync", &["power", "--sleep-sync", v], |p| p.sleep_sync = !p.sleep_sync);
        }
        if switch(ui, row(2), Id::unique("lock-sync"), "Lock all when this one locks", p.lock_sync, true).clicked() {
            let v = if p.lock_sync { "off" } else { "on" };
            self.set_pref("lock sync", &["lock-sync", v], |p| p.lock_sync = !p.lock_sync);
        }
    }

    fn keys_card(&mut self, ui: &mut Ui, r: Rect) {
        let c = card(ui, r, "Keyboard shortcuts");
        let p = self.prefs.clone();
        let top = c.top() - 6.0;
        let r0 = Rect::from_min_size(pos2(c.left(), top), vec2(c.width(), 36.0));
        if switch(ui, r0, Id::unique("hotkeys"), "Use keyboard shortcuts", p.hotkeys_on, true).clicked() {
            let flag = if p.hotkeys_on { "--off" } else { "--on" };
            self.set_pref("keyboard shortcuts", &["hotkeys", flag], |p| p.hotkeys_on = !p.hotkeys_on);
        }
        ui.painter().hline(c.x_range(), top + 36.0, Stroke::new(1.0, LINE));
        let mac = cfg!(target_os = "macos");
        let rows: Vec<(&str, String)> = p
            .hotkeys
            .iter()
            .map(|(what, chord)| (*what, view::chord_label(chord, mac)))
            .chain(std::iter::once(("Always back here", view::chord_label("ctrl+alt+shift+esc", mac))))
            .collect();
        let mut y = top + 50.0;
        for (what, chord) in &rows {
            let col = if p.hotkeys_on || *what == "Always back here" { INK } else { INK2 };
            ui.painter().text(pos2(c.left(), y), Align2::LEFT_CENTER, *what, regular(T_BODY), INK2);
            let cw = c.width() - text_width(ui, what, regular(T_BODY)) - 12.0;
            ui.painter().text(pos2(c.right(), y), Align2::RIGHT_CENTER, fit(ui.painter(), chord, &mono(T_META), cw), mono(T_META), col);
            let rr = Rect::from_min_max(pos2(c.left(), y - 10.0), pos2(c.right(), y + 10.0));
            a11y_text(ui, Id::unique(("hotkey", *what)), rr, Role::Label, &format!("{what}: {chord}"), None);
            y += 22.0;
        }
        if let Some(note) = self.running().and_then(|s| view::hotkeys_note(&s.hotkeys, &s.hotkeys_detail)) {
            para(ui, pos2(c.left(), y), c.width(), &note, regular(T_META), INK2);
        }
    }

    fn hooks_card(&mut self, ui: &mut Ui, r: Rect) {
        let c = card(ui, r, "Run commands");
        let p = self.prefs.clone();
        let label = "Edit settings.toml";
        let bw = button_width(ui, label) - 4.0;
        let b = Rect::from_min_size(pos2(r.right() - 18.0 - bw, r.top() + 12.0), vec2(bw, 28.0));
        let busy = self.busy.contains(&Job::EditSettings);
        if button(ui, b, Id::unique("edit-settings"), label, Look::Quiet, !busy, Some("Open settings.toml to edit the commands")).clicked() {
            self.busy.push(Job::EditSettings);
            self.link.edit_settings();
        }
        let mut y = c.top() + 2.0;
        if p.unreadable {
            p_line(ui, c, y, ERROR, "settings.toml couldn't be read; showing the defaults.");
            y += 22.0;
        }
        if p.hooks.is_empty() {
            let text = "When the pointer arrives or leaves, or a computer connects. None are set yet.";
            para(ui, pos2(c.left(), y - 8.0), c.width(), text, regular(T_BODY), INK2);
            return;
        }
        for (when, cmd) in &p.hooks {
            ui.painter().text(pos2(c.left(), y), Align2::LEFT_CENTER, *when, regular(T_BODY), INK2);
            let kw = text_width(ui, when, regular(T_BODY)) + 12.0;
            let shown = fit(ui.painter(), cmd, &mono(T_META), c.width() - kw);
            widgets::mono_text(ui.painter(), pos2(c.right(), y), Align2::RIGHT_CENTER, cmd, T_META, INK, c.width() - kw);
            let rr = Rect::from_min_max(pos2(c.left(), y - 10.0), pos2(c.right(), y + 10.0));
            let resp = ui.interact(rr, Id::unique(("hook", *when)), Sense::hover());
            if shown != *cmd {
                resp.on_hover_text(cmd);
            }
            a11y_text(ui, Id::unique(("hook-node", *when)), rr, Role::Label, &format!("{when}: {cmd}"), None);
            y += 22.0;
        }
        let meta = format!("Time limit {} s · {}", p.hooks_timeout, if p.hooks_shell { "through the shell" } else { "no shell" });
        ui.painter().text(pos2(c.left(), y + 2.0), Align2::LEFT_CENTER, meta, regular(T_META), INK2);
    }
}

fn p_line(ui: &Ui, c: Rect, y: f32, dot: Color32, text: &str) {
    ui.painter().circle_filled(pos2(c.left() + 3.0, y), 3.0, dot);
    ui.painter().text(pos2(c.left() + 14.0, y), Align2::LEFT_CENTER, fit(ui.painter(), text, &regular(T_META), c.width() - 14.0), regular(T_META), INK);
}
