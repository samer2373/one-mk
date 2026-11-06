//! Two full pages without the desk: Health (`one-mk doctor`, run on open
//! and on Recheck, off the window's thread, plus what the daemon says
//! right now) and Advanced (edge rules, power and locking, gaming mode,
//! keyboard shortcuts, Ctrl and Cmd, commands to run).

use super::*;
use crate::view::{self, Check, Verdict};

/// Doctor results older than this are run again when Health opens.
const STALE: f64 = 30.0;
/// A row of the Crossing edges card.
/// Tall enough for a label with its caption under it.
const EDGE_ROW: f32 = 50.0;
const HELP_ROW: f32 = 50.0;
const MODS: [(&str, &str); 4] = [("none", "None"), ("shift", "Shift"), ("ctrl", "Ctrl"), ("alt", "Alt")];

/// A group of settings, as the system's own settings show them: its title
/// above, quiet, and the rows on one quiet group below it.
pub(super) fn card(ui: &Ui, r: Rect, title: &str) -> Rect {
    let glass = Rect::from_min_max(r.min + vec2(0.0, GROUP_TITLE), r.max);
    widgets::group(ui, glass, R_GROUP);
    let p = ui.painter();
    p.text(r.left_top() + vec2(14.0, 13.0), Align2::LEFT_CENTER, title, semibold(T_BODY), INK2);
    a11y_text(ui, Id::unique(("card", title)), Rect::from_min_size(r.min, vec2(r.width(), GROUP_TITLE)), Role::Heading, title, None);
    Rect::from_min_max(r.min + vec2(18.0, 48.0), r.max - vec2(18.0, 14.0))
}

/// The room a group's title takes above its glass.
const GROUP_TITLE: f32 = 30.0;

/// What the window can do itself about a failed check.
#[derive(Clone, Copy)]
enum Fix {
    Start,
    Pair,
    OpenPermissionGuide,
    OpenPermissionSettings,
    OpenFirewallSettings,
    Update,
    /// The steps for the desktop's own permission dialog (not macOS).
    DesktopGuide,
}

fn verdict_icon(p: &Painter, c: Pos2, v: Verdict) {
    match v {
        Verdict::Pass => widgets::draw_check(p, c, 12.0, INK2, 1.0),
        // Pending (a step to take, or on its way): an open ring with a
        // clock hand, in ink. Volt stays for actions, never status.
        Verdict::Skip => {
            p.circle_stroke(c, 6.0, Stroke::new(1.5, INK2));
            p.line_segment([c, c + vec2(0.0, -3.5)], Stroke::new(1.5, INK2));
            p.line_segment([c, c + vec2(2.5, 1.5)], Stroke::new(1.5, INK2));
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

/// Whether a line the daemon says now is still pending, and if so whether
/// it waits on the person (`true`: a permission to answer) or on oneMK.
fn pending_kind(line: &str) -> Option<bool> {
    let l = line.to_lowercase();
    if ["waiting for your approval", "is asking", "choose allow", "allow one-mk", "not allowed"].iter().any(|w| l.contains(w)) {
        Some(true)
    } else if ["being set up", "starting", "setting up"].iter().any(|w| l.contains(w)) {
        Some(false)
    } else {
        None
    }
}

/// Whether a row's written fix shows: always, unless the window's own
/// button does that very thing (Start, Pair, Allow...).
fn shows_fix_text(here: &Option<(&'static str, Fix)>) -> bool {
    matches!(here, None | Some((_, Fix::OpenPermissionGuide | Fix::DesktopGuide)))
}

/// One line of Health: a check, or something the daemon says right now
/// (no title then). `k` is the check's place, for its buttons.
struct Item {
    title: String,
    detail: String,
    fix: Option<String>,
    verdict: Verdict,
    here: Option<(&'static str, Fix)>,
    k: usize,
}

fn text_height(ui: &Ui, text: &str, width: f32) -> f32 {
    let f = regular(T_BODY);
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), f.clone(), INK, width);
    job.sections[0].format.line_height = Some((f.size * LEADING).round());
    ui.painter().layout_job(job).size().y
}

/// A "Needs you" row's height: title, what is wrong, what to do.
fn item_height(ui: &Ui, i: &Item, width: f32) -> f32 {
    let title = if i.title.is_empty() { 0.0 } else { 24.0 };
    let fix = match (&i.here, &i.fix) {
        (here, Some(f)) if shows_fix_text(here) => text_height(ui, f, width) + 6.0,
        _ => 0.0,
    };
    (12.0 + title + text_height(ui, &i.detail, width) + fix + 14.0).max(if i.here.is_some() || i.fix.is_some() { 56.0 } else { 44.0 })
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
        match c.action.as_deref() {
            Some("Start") if self.off() => return Some(("Start", Fix::Start)),
            Some("Pair") => return Some(("Pair a computer", Fix::Pair)),
            Some("OpenPermissionGuide") => return Some(("Allow sharing", Fix::OpenPermissionGuide)),
            Some("OpenPermissionSettings") => return Some(("Open permissions", Fix::OpenPermissionSettings)),
            Some("OpenFirewallSettings") => return Some(("Open firewall settings", Fix::OpenFirewallSettings)),
            _ => {}
        }
        match c.id.as_str() {
            "updates" if c.fix.as_deref() == Some("update-now") => Some(("Update now", Fix::Update)),
            "daemon" if self.off() => Some(("Start oneMK", Fix::Start)),
            // Pairing works with oneMK off too; it connects once started.
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
            return vec![(INK2, "Sharing is stopped. Choose Start to turn it on.".into())];
        };
        let mut out: Vec<(Color32, String)> = s.input_lines().into_iter().map(|line| (
            if line.severity == one_mk_ctl::status::Severity::Problem { ERROR } else { INK2 }, line.detail,
        )).collect();
        if out.is_empty() {
            out.push((INK2, "Mouse and keyboard sharing works: the desktop allowed it.".into()));
        }
        if let Some(h) = s.hotkeys_line() {
            out.push((INK2, h));
        }
        out
    }

    /// Health: one line that says whether everything works, then what
    /// needs the person (each with the button that fixes it, when the
    /// window can), how each paired computer answers, and what works. A
    /// check about something this computer doesn't use isn't shown.
    pub(super) fn health(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        let running = self.busy.contains(&Job::Doctor);
        let checks: Option<Vec<Check>> = match &self.doctor {
            // A neutral check shows only when there is something to do
            // about it (not paired yet: Pair); the others say nothing new.
            Some((Ok(c), _)) => Some(c.iter().filter(|c| c.verdict != Verdict::Skip || self.window_fix(c).is_some()).cloned().collect()),
            _ => None,
        };
        // What needs the person: the daemon's own problems first (they are
        // happening now), then the checks, worst first.
        // Off, the checks say so themselves (with Start).
        let live = if self.running().is_some() { self.live_lines() } else { vec![] };
        let mut needs: Vec<Item> = live.iter().filter(|(c, _)| *c == ERROR).map(|(_, l)| Item { title: String::new(), detail: l.clone(), fix: None, verdict: Verdict::Fail, here: None, k: usize::MAX }).collect();
        // A line that waits on the person (a permission dialog to answer)
        // or on oneMK (still setting up) is pending: never a tick.
        // On a Mac the guide walks through the permission; elsewhere the
        // dialog is the desktop's own, so the action is to check again.
        let ask = if cfg!(target_os = "macos") { Some(("Open guide", Fix::OpenPermissionGuide)) } else { Some(("Open guide", Fix::DesktopGuide)) };
        let mut works: Vec<Item> = vec![];
        // The daemon's shortcut line says what the "Keyboard shortcuts"
        // check says: one row, not two.
        let has_hotkeys_check = checks.as_ref().is_some_and(|c| c.iter().any(|c| c.id == "hotkeys"));
        let hotkeys_line = self.running().and_then(|s| s.hotkeys_line());
        for (c, l) in live.iter().filter(|(c, _)| *c != ERROR) {
            let _ = c;
            if has_hotkeys_check && hotkeys_line.as_deref() == Some(l.as_str()) {
                continue;
            }
            match pending_kind(l) {
                Some(asks) => needs.push(Item { title: if asks { "Pending permission".into() } else { "Setting up".into() }, detail: l.clone(), fix: None, verdict: Verdict::Skip, here: if asks { ask } else { None }, k: usize::MAX }),
                None => works.push(Item { title: String::new(), detail: l.clone(), fix: None, verdict: Verdict::Pass, here: None, k: usize::MAX }),
            }
        }
        if let Some(checks) = &checks {
            for (k, c) in checks.iter().enumerate() {
                // Every row that isn't a pass carries an action: the
                // window's own fix, a command to copy, or Recheck.
                let here = self.window_fix(c).or_else(|| match (&c.fix, c.verdict) {
                    (_, Verdict::Pass) => None,
                    (Some(f), _) if view::fix_command(f).1 => None,
                    _ if matches!(c.id.as_str(), "input" | "permissions" | "accessibility") => Some(("Open guide", if cfg!(target_os = "macos") { Fix::OpenPermissionGuide } else { Fix::DesktopGuide })),
                    // Nothing the window can do here: the fix is in words,
                    // and Recheck (once, at the top) checks again.
                    _ => None,
                });
                let item = Item { title: c.title.clone(), detail: c.detail.clone(), fix: c.fix.clone(), verdict: c.verdict, here, k };
                if c.verdict == Verdict::Pass { works.push(item) } else { needs.push(item) }
            }
        }
        // Worst first; steps still to take (neutral) last.
        needs.sort_by_key(|i| match i.verdict {
            Verdict::Fail => 0,
            Verdict::Warn => 1,
            _ => 2,
        });

        let w = (rect.width() - 48.0).min(960.0);
        // Inset as Settings is: the column starts under the page title.
        let x = rect.left() + 24.0;
        let area = Rect::from_min_max(pos2(rect.left(), rect.top() + 8.0), rect.max - vec2(0.0, 8.0));
        let mut child = ui.new_child(UiBuilder::new().max_rect(area));
        child.set_clip_rect(area);
        let mut fix_now = None;
        let mut copy = None;
        let mut recheck = false;
        let copied = self.copied;
        egui::ScrollArea::vertical().id_salt("health-page").auto_shrink([false, false]).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let top = ui.cursor().top();
            let col = |ui: &mut Ui, h: f32| -> Rect {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
                Rect::from_min_size(pos2(x, r.top()), vec2(w, h))
            };
            let _ = top;

            // The verdict, in one line, with Recheck.
            let fails = needs.iter().filter(|i| i.verdict == Verdict::Fail).count();
            let steps_only = !needs.is_empty() && needs.iter().all(|i| i.verdict == Verdict::Skip);
            let (headline, tone) = if running && checks.is_none() {
                ("Checking this computer…".to_owned(), INK2)
            } else if needs.is_empty() && checks.is_some() {
                ("Everything is working".to_owned(), OK)
            } else if needs.is_empty() {
                ("Not checked yet".to_owned(), INK2)
            } else {
                // The count names the groups below it: problems under Needs
                // you, the rest under Pending.
                let pending = needs.iter().filter(|i| i.verdict == Verdict::Skip).count();
                let problems = needs.len() - pending;
                let asks = needs.iter().any(|i| i.verdict == Verdict::Skip && matches!(i.here, Some((_, Fix::OpenPermissionGuide | Fix::DesktopGuide))));
                let p_words = match (pending, asks) {
                    (1, true) => "1 awaiting permission".to_owned(),
                    (n, true) => format!("{n} awaiting you"),
                    (n, false) => format!("{n} pending"),
                };
                let tone = if fails > 0 { ERROR } else { INK };
                match (problems, steps_only) {
                    (_, true) => (p_words, INK),
                    (n, _) if pending == 0 => (format!("{n} {} attention", if n == 1 { "needs" } else { "need" }), tone),
                    (n, _) => (format!("{n} {} attention · {p_words}", if n == 1 { "needs" } else { "need" }), tone),
                }
            };
            let sub = match &self.doctor {
                Some((Err(e), _)) => format!("The checks didn't run: {e}"),
                Some((Ok(_), at)) => {
                    let ago = (now - at).max(0.0);
                    let when = if ago < 60.0 { "just now".to_owned() } else if ago < 3600.0 { format!("{} min ago", (ago / 60.0) as u32) } else { format!("{} h ago", (ago / 3600.0) as u32) };
                    format!("{} checks pass · checked {when}", works.iter().filter(|i| i.k != usize::MAX).count())
                }
                None => String::new(),
            };
            let hero = col(ui, 92.0);
            widgets::group(ui, hero, R_GROUP);
            let p = ui.painter().clone();
            let medal = pos2(hero.left() + 46.0, hero.center().y);
            p.circle_filled(medal, 22.0, tone.gamma_multiply(0.16));
            p.circle_stroke(medal, 22.0, Stroke::new(1.0, tone.gamma_multiply(0.5)));
            if running && checks.is_none() {
                let turn = if motion::reduced() { 0.0 } else { (now * 5.0) as f32 };
                widgets::draw_refresh(&p, Rect::from_center_size(medal, vec2(18.0, 18.0)), INK2, turn);
                ui.ctx().request_repaint_after_secs(1.0 / 30.0);
            } else if tone == OK {
                widgets::draw_check(&p, medal, 18.0, OK, 1.0);
            } else if needs.is_empty() {
                p.circle_filled(medal, 3.0, INK2);
            } else if steps_only {
                p.text(medal, Align2::CENTER_CENTER, needs.len().to_string(), semibold(T_TITLE + 1.0), tone);
            } else {
                p.text(medal, Align2::CENTER_CENTER, "!", semibold(T_TITLE + 3.0), tone);
            }
            let bw = button_width(ui, "Recheck");
            let tx = hero.left() + 84.0;
            let tw = hero.width() - 84.0 - bw - 36.0;
            p.text(pos2(tx, hero.center().y - (if sub.is_empty() { 0.0 } else { 11.0 })), Align2::LEFT_CENTER, fit(&p, &headline, &semibold(T_TITLE + 3.0), tw), semibold(T_TITLE + 3.0), INK);
            if !sub.is_empty() {
                p.text(pos2(tx, hero.center().y + 13.0), Align2::LEFT_CENTER, fit(&p, &sub, &regular(T_BODY), tw), regular(T_BODY), INK2);
            }
            let b = Rect::from_min_size(pos2(hero.right() - 18.0 - bw, hero.center().y - 16.0), vec2(bw, 32.0));
            let label = if running { "Checking…" } else { "Recheck" };
            if button(ui, b, Id::unique("recheck"), label, Look::Quiet, !running, Some("Run the checks again")).clicked() {
                recheck = true;
            }
            a11y_text(ui, Id::unique("health-summary"), hero, Role::Status, &format!("{headline}. {sub}"), Some(Live::Polite));
            ui.add_space(14.0);

            // What needs the person (problems), then what is pending (a step
            // to take, or something still on its way): two groups.
            let (pending, problems): (Vec<&Item>, Vec<&Item>) = needs.iter().partition(|i| i.verdict == Verdict::Skip);
            for (group_title, needs) in [("Needs you", problems), ("Pending", pending)] {
            if !needs.is_empty() {
                let inner = w - 36.0 - 26.0;
                let bw = needs.iter().map(|i| match (&i.here, &i.fix) {
                    (Some((label, _)), _) => button_width(ui, label),
                    (None, Some(f)) if view::fix_command(f).1 => button_width(ui, "Copy command"),
                    _ => 0.0,
                }).fold(0.0, f32::max);
                let text_w = inner - if bw > 0.0 { bw + 16.0 } else { 0.0 };
                let heights: Vec<f32> = needs.iter().map(|i| item_height(ui, i, text_w)).collect();
                let h = 52.0 + heights.iter().sum::<f32>() + 6.0;
                let r = col(ui, h);
                card(ui, r, group_title);
                let mut y = r.top() + 48.0;
                for (n, (i, ih)) in needs.iter().zip(&heights).enumerate() {
                    let row = Rect::from_min_size(pos2(r.left() + 18.0, y), vec2(w - 36.0, *ih));
                    let p = ui.painter().clone();
                    verdict_icon(&p, pos2(row.left() + 8.0, row.top() + 22.0), i.verdict);
                    let tx = row.left() + 26.0;
                    let mut ty = row.top() + 12.0;
                    if !i.title.is_empty() {
                        p.text(pos2(tx, ty + 10.0), Align2::LEFT_CENTER, &i.title, semibold(T_BODY), INK);
                        ty += 24.0;
                    }
                    // What is wrong in ink; what to do, quieter, under it.
                    ty += widgets::para_code(ui, pos2(tx, ty), text_w, &i.detail, regular(T_BODY), INK);
                    let fix_text = i.fix.clone().filter(|_| shows_fix_text(&i.here));
                    if let Some(f) = &fix_text {
                        widgets::para_code(ui, pos2(tx, ty + 6.0), text_w, f, regular(T_BODY), INK2);
                    }
                    let br = Rect::from_min_size(pos2(row.right() - bw, row.top() + 6.0), vec2(bw, 32.0));
                    if let Some((label, job)) = &i.here {
                        let resp = button(ui, br, Id::unique(("fix-here", i.k)), label, Look::Quiet, !running, None);
                        if matches!(job, Fix::DesktopGuide) {
                            // The desktop's own dialog: the steps, beside the button.
                            egui::Popup::menu(&resp).width(320.0).align(egui::RectAlign::BOTTOM_END).show(|ui| {
                                ui.set_max_width(320.0);
                                let steps = [
                                    "1. Look for your desktop's sharing window (“Remote desktop” or “Input capture”). It can be behind other windows; check the taskbar.",
                                    "2. Choose Allow (or Share). oneMK only asks for the mouse and keyboard.",
                                    "3. Back here, choose Recheck. Dismissed it? Stop and start oneMK to be asked again.",
                                ];
                                for t in steps {
                                    ui.add(egui::Label::new(egui::RichText::new(t).font(regular(T_BODY)).color(INK_HOVER)).wrap());
                                    ui.add_space(4.0);
                                }
                            });
                        } else if resp.clicked() {
                            fix_now = Some(*job);
                        }
                    } else if let Some(f) = &i.fix {
                        let (cmd, is_cmd) = view::fix_command(f);
                        if is_cmd {
                            let word = if copied.as_ref().is_some_and(|(k, t)| *k == i.k && now - t < 1.5) { "Copied" } else { "Copy command" };
                            if button(ui, br, Id::unique(("copy-fix", i.k)), word, Look::Quiet, true, Some(&format!("Copy the fix for {}: {cmd}", i.title))).clicked() {
                                copy = Some((i.k, cmd));
                            }
                        }
                    }
                    let state = if group_title == "Pending" { "pending" } else { "needs you" };
                    let label = if i.title.is_empty() { format!("{state}: {}", i.detail) } else { format!("{}: {state}. {}", i.title, i.detail) };
                    a11y_text(ui, Id::unique(("need", group_title, n)), row, Role::ListItem, &label, None);
                    if n + 1 < needs.len() {
                        ui.painter().hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, LINE));
                    }
                    y += ih;
                }
                ui.add_space(14.0);
            }
            }

            // How each paired computer answers.
            let before = ui.cursor().top();
            let links = self.latency_rows(ui, x, before, w);
            if links > 0.0 {
                ui.allocate_exact_size(vec2(ui.available_width(), links + 14.0), Sense::hover());
            }

            // What works: one quiet line each.
            if !works.is_empty() {
                let text_w = w - 36.0 - 26.0;
                let line = |i: &Item| if i.title.is_empty() { i.detail.clone() } else { i.title.clone() };
                // A line from the daemon (no title) is its detail alone,
                // wrapped like the body; a check is its title and a smaller
                // detail.
                let heights: Vec<f32> = works.iter().map(|i| {
                    if i.title.is_empty() {
                        10.0 + text_height(ui, &i.detail, text_w).max(20.0) + 10.0
                    } else {
                        10.0 + 20.0 + text_height(ui, &i.detail, text_w) + 2.0 + 10.0
                    }
                }).collect();
                let h = 52.0 + heights.iter().sum::<f32>() + 6.0;
                let r = col(ui, h);
                card(ui, r, "Passed");
                let mut y = r.top() + 44.0;
                for (n, (i, ih)) in works.iter().zip(&heights).enumerate() {
                    let p = ui.painter().clone();
                    let tx = r.left() + 18.0 + 26.0;
                    widgets::draw_check(&p, pos2(r.left() + 26.0, y + 20.0), 12.0, OK, 1.0);
                    if i.title.is_empty() {
                        widgets::para_code(ui, pos2(tx, y + 10.0), text_w, &i.detail, regular(T_BODY), INK);
                    } else {
                        p.text(pos2(tx, y + 20.0), Align2::LEFT_CENTER, line(i), regular(T_BODY), INK);
                        widgets::para_code(ui, pos2(tx, y + 32.0), text_w, &i.detail, regular(T_META), INK3);
                    }
                    a11y_text(ui, Id::unique(("works", n)), Rect::from_min_size(pos2(r.left(), y), vec2(w, *ih)), Role::ListItem, &if i.title.is_empty() { i.detail.clone() } else { format!("{}: works. {}", line(i), i.detail) }, None);
                    y += ih;
                }
                ui.add_space(8.0);
            }
        });
        if recheck {
            self.recheck();
        }
        if let Some((k, text)) = copy {
            ui.ctx().copy_text(text);
            self.copied = Some((k, now));
            ui.ctx().request_repaint_after_secs(1.6);
        }
        match fix_now {
            Some(Fix::Update) => self.begin_update(),
            Some(Fix::Start) if !self.busy.contains(&Job::Start) => {
                self.busy.push(Job::Start);
                self.link.start();
            }
            Some(Fix::Pair) => self.start_pairing(now),
            Some(Fix::OpenPermissionGuide) => {
                let lines = self.running().map(|status| status.input.clone()).unwrap_or_default();
                self.guide.start(&crate::guide::Permission::ALL, Box::new(crate::guide::StatusPermissions { lines }), now);
            }
            Some(Fix::OpenFirewallSettings) => ui.ctx().open_url(egui::OpenUrl::new_tab("x-apple.systempreferences:com.apple.Network-Settings.extension?Firewall")),
            Some(Fix::OpenPermissionSettings) => ui.ctx().open_url(egui::OpenUrl::new_tab("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")),
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
        let peers = s.map_or(0, |status| status.peers.len());
        let heights = [
            48.0 + 5.0 * EDGE_ROW + 14.0,
            48.0 + 3.0 * HELP_ROW - 6.0 + 14.0,
            48.0 + HELP_ROW - 6.0 + 14.0,
            48.0 + 62.0 + super::settings_controls::SHORTCUT_ROW * self.prefs.hotkeys.len() as f32 + 10.0 + if s.is_some_and(|status| status.hotkeys != "on" && crate::view::hotkeys_note(&status.hotkeys, &status.hotkeys_detail).is_some()) { 130.0 } else { 76.0 },
            48.0 + 22.0 + (super::swap_row_h(col_w - 36.0) * peers as f32).max(40.0) + 14.0,
            48.0 + 4.0 * super::settings_controls::HOOK_H + 60.0,
            48.0 + 96.0 + 14.0,
            48.0 + 260.0 + 14.0,
            self.updates_card_h(),
        ];
        // Two columns: crossing, power, games, pointer and layout, and
        // updates on the left; keys and commands on the right. One column
        // below 700 points.
        let mut at = [(0usize, 0.0f32); 9];
        let mut ys = [0.0f32; 2];
        // Updates first: it is the one card people come to Settings for.
        for k in [8, 0, 1, 2, 3, 4, 5, 6, 7] {
            let h = &heights[k];
            let col = if two { usize::from((3..7).contains(&k)) } else { 0 };
            at[k] = (col, ys[col]);
            ys[col] += h + gap;
        }
        let total = ys[0].max(ys[1]) - gap;
        let mut child = ui.new_child(UiBuilder::new().max_rect(area.expand2(vec2(6.0, 0.0))));
        child.set_clip_rect(area.expand2(vec2(8.0, 4.0)));
        let scroll = egui::ScrollArea::vertical().id_salt("advanced").auto_shrink([false, false]);
        #[cfg(debug_assertions)]
        let scroll = std::env::var("ONE_MK_APP_SCROLL").ok().and_then(|value| value.parse::<f32>().ok()).map_or(scroll.clone(), |offset| scroll.vertical_scroll_offset(offset));
        scroll.show(&mut child, |ui| {
            let (all, _) = ui.allocate_exact_size(vec2(area.width(), total + 44.0), Sense::hover());
            let r = |k: usize| Rect::from_min_size(pos2(all.left() + at[k].0 as f32 * (col_w + gap), all.top() + at[k].1), vec2(col_w, heights[k]));
            // Updates first: first on the page, first in Tab order.
            self.updates_card(ui, r(8));
            self.edges_card(ui, r(0));
            self.power_card(ui, r(1));
            let c = card(ui, r(2), "Games");
            self.gaming_row(ui, Rect::from_min_size(pos2(c.left(), c.top() - 6.0), vec2(c.width(), HELP_ROW)));
            self.keys_card(ui, r(3));
            let c = card(ui, r(4), "Ctrl and Cmd");
            ui.painter().text(pos2(c.left(), c.top() + 2.0), Align2::LEFT_CENTER, fit(ui.painter(), "Auto swaps them only between a Mac and a PC.", &regular(T_META), c.width()), regular(T_META), INK2);
            self.swaps(ui, Rect::from_min_max(pos2(c.left(), c.top() + 18.0), c.max));
            self.hooks_card(ui, r(5));
            self.typing_card(ui, r(6));
            let c = card(ui, r(7), "Pointer and layout");
            self.extra_controls(ui, c);
            // Credits for what the window draws and isn't ours: last on the
            // page, and last in Tab order.
            let credit = "The 3D mouse is “Logitech G502 x lightspeed” by Okopchi, CC BY 4.0, changed for oneMK.";
            let credit_at = pos2(all.left() + 4.0, all.bottom() - 16.0);
            let shown = fit(ui.painter(), credit, &regular(T_META), all.width() - 8.0);
            let link = ui.interact(widgets::hit_rect(Rect::from_min_size(credit_at - vec2(0.0, 9.0), vec2(text_width(ui, &shown, regular(T_META)), 18.0))), Id::unique("credit-mouse"), Sense::click());
            let lr = link.rect;
            ui.painter().text(credit_at, Align2::LEFT_CENTER, &shown, regular(T_META), if link.hovered() { INK } else { INK3 });
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

    fn edges_card(&mut self, ui: &mut Ui, r: Rect) {
        let c = card(ui, r, "Crossing edges");
        let mut p = self.prefs.clone();
        let scope_id = Id::unique("edge-scope");
        let mut side = ui.ctx().data(|data| data.get_temp::<String>(scope_id)).unwrap_or_default();
        let track = Rect::from_min_size(c.min, vec2(c.width(), 28.0));
        if let Some(chosen) = self.settings_segment(ui, track, "Crossing edge", &[("", "Shared"), ("left", "Left"), ("right", "Right"), ("top", "Top"), ("bottom", "Bottom")], &side) { side = chosen; }
        ui.ctx().data_mut(|data| data.insert_temp(scope_id, side.clone()));
        if let Some((_, rules)) = p.edge_sides.iter().find(|(name, _)| *name == side) {
            p.edge_corner = rules.corner;
            p.edge_dwell_ms = rules.dwell;
            p.edge_modifier = rules.modifier.clone();
            p.edge_fullscreen = rules.fullscreen;
        }
        let top = c.top() + 34.0;
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
            label_help(ui, rr, Id::unique(("edge-label", key)), label, &format!("edge-{key}"), true, 72.0 + uw + 12.0);
            let fr = Rect::from_min_size(pos2(rr.right() - 72.0 - uw, rr.center().y - 13.0), vec2(72.0, 26.0));
            ui.painter().text(pos2(rr.right(), rr.center().y), Align2::RIGHT_CENTER, unit, regular(T_META), INK2);
            let a11y = format!("{label}, in {}", if unit == "px" { "pixels (0: off)" } else { "milliseconds (0: off)" });
            if let Some(text) = self.edit_field(ui, fr, &format!("edge-{key}"), &cur.to_string(), "0", &a11y, true, true) {
                match text.parse::<u32>() {
                    Ok(n) if n <= max => {
                        let v = n.to_string();
                        self.edge_setting(&side, flag, &v);
                    }
                    _ => self.toast_error(format!("{label}: type a whole number from 0 to {max}."), None, self.now),
                }
            }
        }
        let rr = row(2);
        let word = "Hold a key to cross";
        let lw = text_width(ui, word, regular(T_BODY)) + 16.0 + 26.0;
        let seg_w = 200.0f32.min(rr.width() - lw);
        label_help(ui, rr, Id::unique("edge-mod-label"), word, "edge-mod", true, seg_w + 12.0);
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
        };
        if let Some(k) = pills.show(ui, self.arrowed) {
            let v = MODS[k].0;
            self.edge_setting(&side, "--modifier", v);
        }
        let r = switch_help(ui, row(3), Id::unique("edge-fullscreen"), "Stay in fullscreen windows", "edge-fullscreen", p.edge_fullscreen, true);
        if r.clicked() {
            let v = if p.edge_fullscreen { "off" } else { "on" };
            self.edge_setting(&side, "--fullscreen-guard", v);
        }
        let reset = Rect::from_min_size(pos2(c.left(), row(4).top()), vec2(c.width(), 28.0));
        if !side.is_empty() && button(ui, reset, Id::unique("edge-reset-side"), "Use shared rules", Look::Quiet, true, None).clicked() {
            self.settings_command("the edge rules", vec!["edge".into(), "--side".into(), side.clone(), "--reset".into()]);
        }
    }

    fn power_card(&mut self, ui: &mut Ui, r: Rect) {
        let c = card(ui, r, "Power and locking");
        let p = self.prefs.clone();
        let row_h = HELP_ROW;
        let top = c.top() - 6.0;
        let row = |k: usize| Rect::from_min_size(pos2(c.left(), top + row_h * k as f32), vec2(c.width(), row_h));
        let r0 = switch_help(ui, row(0), Id::unique("block-saver"), "Keep computers I control awake", "block-saver", p.block_screensaver, true);
        if r0.clicked() {
            let v = if p.block_screensaver { "off" } else { "on" };
            self.set_pref("keeping computers awake", &["power", "--block-screensaver", v], |p| p.block_screensaver = !p.block_screensaver);
        }
        if switch_help(ui, row(1), Id::unique("sleep-sync"), "Sleep displays together", "sleep-sync", p.sleep_sync, true).clicked() {
            let v = if p.sleep_sync { "off" } else { "on" };
            self.set_pref("sleep sync", &["power", "--sleep-sync", v], |p| p.sleep_sync = !p.sleep_sync);
        }
        if switch_help(ui, row(2), Id::unique("lock-sync"), "Lock all when this one locks", "lock-sync", p.lock_sync, true).clicked() {
            let v = if p.lock_sync { "off" } else { "on" };
            self.set_pref("lock sync", &["lock-sync", v], |p| p.lock_sync = !p.lock_sync);
        }
    }

    fn keys_card(&mut self, ui: &mut Ui, r: Rect) {
        let c = card(ui, r, "Keyboard shortcuts");
        let row = Rect::from_min_size(c.min, vec2(c.width(), HELP_ROW));
        if switch_help(ui, row, Id::unique("hotkeys"), "Use keyboard shortcuts", "hotkeys", self.prefs.hotkeys_on, true).clicked() {
            self.set_pref("keyboard shortcuts", &["hotkeys", if self.prefs.hotkeys_on { "--off" } else { "--on" }], |prefs| prefs.hotkeys_on = !prefs.hotkeys_on);
        }
        self.shortcut_controls(ui, Rect::from_min_max(c.min + vec2(0.0, 62.0), c.max));
    }

    fn hooks_card(&mut self, ui: &mut Ui, r: Rect) {
        let c = card(ui, r, "Run commands");
        self.hook_controls(ui, c);
    }
}
