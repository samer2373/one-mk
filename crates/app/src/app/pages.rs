
//! Health, a full page without the desk: `one-mk doctor`, run on open and
//! on Check again, off the window's thread, plus what the daemon says right
//! now. A banner says in one line whether all is well; the cards under it
//! hold what needs the person (Needs you, Pending), how each computer
//! answers (Connections) and what passed. Settings is in `settings_page`.

use super::*;
use crate::view::{self, Check, Verdict};

/// Doctor results older than this are run again when Health opens.
const STALE: f64 = 30.0;

/// The room a group's label takes above its card (see [`group_card`]).
pub(super) const GROUP_LABEL: f32 = 28.0;

/// A group of rows as Health and Settings draw it: its label (13, 600,
/// `ink2`) above the card (radius 14, padding 0 18) that is `rows_h` tall,
/// from `at` and `w` wide. Returns the room for the rows.
pub(super) fn group_card(ui: &Ui, at: Pos2, w: f32, title: &str, rows_h: f32) -> Rect {
    let card = Rect::from_min_size(at + vec2(0.0, GROUP_LABEL), vec2(w, rows_h));
    widgets::group(ui, card, R_GROUP);
    ui.painter().text(at + vec2(2.0, 12.0), Align2::LEFT_CENTER, title, semibold(T_BODY), pal().ink2);
    a11y_text(ui, Id::unique(("group", title)), Rect::from_min_size(at, vec2(w, GROUP_LABEL)), Role::Heading, title, None);
    Rect::from_min_size(card.min + vec2(18.0, 0.0), vec2(w - 36.0, rows_h))
}

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

fn current_check(mut check: Check, status: Option<&Status>) -> Option<Check> {
    if let Some(status) = status {
        if check.id == "pairing" && !status.peers.is_empty() {
            check.verdict = Verdict::Pass;
            check.detail = if status.peers.len() == 1 { "1 paired computer is saved.".to_owned() } else { format!("{} paired computers are saved.", status.peers.len()) };
            check.fix = None;
            check.action = None;
        }
        if matches!(check.id.as_str(), "input" | "permissions" | "accessibility") && status.input_lines().iter().any(|line| pending_kind(&line.detail).is_some()) {
            return None;
        }
    }
    Some(check)
}

/// The mark a check's verdict gets: always a shape and a colour. ● passed,
/// ◆ needs a look, ■ a problem, ○ waiting.
fn verdict_mark(v: Verdict) -> widgets::Mark {
    match v {
        Verdict::Pass => widgets::Mark::Ok,
        Verdict::Skip => widgets::Mark::Off,
        Verdict::Warn => widgets::Mark::Warn,
        Verdict::Fail => widgets::Mark::Error,
    }
}

/// The width of a "Needs you" row's button: the window's own fix, or
/// copying the command; 0 for none.
fn fix_button_w(ui: &Ui, i: &Item) -> f32 {
    match (&i.here, &i.fix) {
        (Some((label, _)), _) => button_width(ui, label),
        (None, Some(f)) if view::fix_command(f).1 => button_width(ui, "Copy command"),
        _ => 0.0,
    }
}

/// A "Needs you" or "Pending" row's height `w` wide (the text's room):
/// what it is about, what is wrong (600), what to do.
fn need_height(ui: &Ui, i: &Item, w: f32) -> f32 {
    let title = if i.title.is_empty() { 0.0 } else { 18.0 };
    let fix = match (&i.here, &i.fix) {
        (here, Some(f)) if shows_fix_text(here) => 4.0 + widgets::para_code_h(ui, w, f, regular(T_BODY)),
        _ => 0.0,
    };
    let min = if i.here.is_some() || i.fix.is_some() { 56.0 } else { 48.0 };
    (14.0 + title + widgets::para_code_h(ui, w, &i.detail, semibold(T_BODY)) + fix + 14.0).max(min)
}

/// A passed row's height `w` wide: its name, and what it found under it.
fn passed_height(ui: &Ui, i: &Item, w: f32) -> f32 {
    if i.title.is_empty() {
        (14.0 + widgets::para_code_h(ui, w, &i.detail, regular(T_BODY)) + 14.0).max(48.0)
    } else {
        (12.0 + 20.0 + widgets::para_code_h(ui, w, &i.detail, regular(T_META)) + 12.0).max(52.0)
    }
}

/// What a Health row is.
enum Line<'a> {
    Need(&'a Item),
    Peer(usize),
    /// "6 other checks · Passed": opens and closes the list of passed checks.
    Toggle(usize),
    Passed(&'a Item),
}

struct Card<'a> {
    title: &'a str,
    /// Quiet words at the right of the label.
    extra: Option<String>,
    lines: Vec<(Line<'a>, f32)>,
}

/// How the banner at the top of Health looks.
#[derive(Clone, Copy, PartialEq)]
enum Tone {
    /// Something needs a look: ◆ on the warning tint.
    Warn,
    /// A problem: ■ on the warning tint.
    Danger,
    /// All well: ● on a plain card.
    Ok,
    /// Checking, or not checked yet: ○ on a plain card.
    Quiet,
}

impl App {
    /// What Health says under its title: when the checks last ran.
    pub(super) fn health_sub(&self) -> String {
        match &self.doctor {
            Some((Ok(_), at)) => {
                let ago = (self.now - at).max(0.0);
                let when = if ago < 60.0 { "just now".to_owned() } else if ago < 3600.0 { format!("{} min ago", (ago / 60.0) as u32) } else if ago < 86400.0 { format!("{} h ago", (ago / 3600.0) as u32) } else { format!("{} days ago", (ago / 86400.0) as u32) };
                format!("Checked {when}")
            }
            _ => String::new(),
        }
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

    pub(super) fn recheck(&mut self) {
        self.busy.push(Job::Doctor);
        self.link.doctor();
    }

    /// Lines under the Health title that come from the daemon itself.
    fn live_lines(&self) -> Vec<(Color32, String)> {
        let Some(s) = self.running() else {
            return vec![(pal().ink2, "Sharing is stopped. Choose Start to turn it on.".into())];
        };
        let mut out: Vec<(Color32, String)> = s.input_lines().into_iter().map(|line| (
            if line.severity == one_mk_ctl::status::Severity::Problem { pal().danger } else { pal().ink2 }, line.detail,
        )).collect();
        if let Some(h) = s.hotkeys_line() {
            out.push((pal().ink2, h));
        }
        out
    }

    /// Health: one line that says whether everything works, then what
    /// needs the person (each with the button that fixes it, when the
    /// window can), how each paired computer answers, and what works. A
    /// check about something this computer doesn't use isn't shown.
    pub(super) fn health_checks(&self) -> Option<Vec<Check>> {
        let (report, _) = self.doctor.as_ref()?;
        let checks = report.as_ref().ok()?;
        Some(checks.iter().filter_map(|check| {
            let check = current_check(check.clone(), self.running())?;
            (check.verdict != Verdict::Skip || self.window_fix(&check).is_some()).then_some(check)
        }).collect())
    }

    /// Health: a banner that says whether everything works, then cards:
    /// what needs the person (each with the button that fixes it, when the
    /// window can), what is pending, how each paired computer answers, and
    /// what passed (a row that opens the list). A check about something
    /// this computer doesn't use isn't shown.
    pub(super) fn health(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        let running = self.busy.contains(&Job::Doctor);
        let checks: Option<Vec<Check>> = match &self.doctor {
            // A neutral check shows only when there is something to do
            // about it (not paired yet: Pair); the others say nothing new.
            Some((Ok(_), _)) => self.health_checks(),
            _ => None,
        };
        // What needs the person: the daemon's own problems first (they are
        // happening now), then the checks, worst first.
        // Off, the checks say so themselves (with Start).
        let live = if self.running().is_some() { self.live_lines() } else { vec![] };
        let mut needs: Vec<Item> = live.iter().filter(|(c, _)| *c == pal().danger).map(|(_, l)| Item { title: String::new(), detail: l.clone(), fix: None, verdict: Verdict::Fail, here: None, k: usize::MAX }).collect();
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
        for (_, l) in live.iter().filter(|(c, _)| *c != pal().danger) {
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
                // window's own fix, a command to copy, or Check again.
                let here = self.window_fix(c).or_else(|| match (&c.fix, c.verdict) {
                    (_, Verdict::Pass) => None,
                    (Some(f), _) if view::fix_command(f).1 => None,
                    _ if matches!(c.id.as_str(), "input" | "permissions" | "accessibility") => Some(("Open guide", if cfg!(target_os = "macos") { Fix::OpenPermissionGuide } else { Fix::DesktopGuide })),
                    // Nothing the window can do here: the fix is in words,
                    // and Check again (once, at the top) checks again.
                    _ => None,
                });
                let item = Item { title: c.title.clone(), detail: c.detail.clone(), fix: c.fix.clone(), verdict: c.verdict, here, k };
                if c.verdict == Verdict::Pass { works.push(item) } else { needs.push(item) }
            }
        }
        // A permission the desktop is asking for now is the same thing as
        // doctor's "not allowed yet": one row (Pending), not two.
        if needs.iter().any(|i| i.title == "Pending permission") {
            needs.retain(|i| !(i.title == "Permissions" && i.verdict != Verdict::Fail));
        }
        // Worst first; steps still to take (neutral) last.
        needs.sort_by_key(|i| match i.verdict {
            Verdict::Fail => 0,
            Verdict::Warn => 1,
            _ => 2,
        });
        let (pending, problems): (Vec<&Item>, Vec<&Item>) = needs.iter().partition(|i| i.verdict == Verdict::Skip);
        let peers: Vec<PeerStatus> = self.running().map(|s| s.peers.clone()).unwrap_or_default();

        // ---- the banner: the verdict, in a line, with Check again ----
        let fails = needs.iter().filter(|i| i.verdict == Verdict::Fail).count();
        let steps_only = !needs.is_empty() && needs.iter().all(|i| i.verdict == Verdict::Skip);
        let didnt_run = match &self.doctor {
            Some((Err(e), _)) => format!("The checks didn't run: {e}"),
            _ => String::new(),
        };
        let (bold, rest, tone) = if running && checks.is_none() {
            ("Checking this computer…".to_owned(), String::new(), Tone::Quiet)
        } else if needs.is_empty() && checks.is_some() {
            ("Checks pass".to_owned(), String::new(), Tone::Ok)
        } else if needs.is_empty() {
            ("Not checked yet".to_owned(), didnt_run.clone(), Tone::Quiet)
        } else {
            // The count names the groups below it: problems under Needs
            // you, the rest under Pending.
            let n_pending = pending.len();
            let n = problems.len();
            let asks = pending.iter().any(|i| matches!(i.here, Some((_, Fix::OpenPermissionGuide | Fix::DesktopGuide))));
            let p_words = match (n_pending, asks) {
                (1, true) => "1 awaiting permission".to_owned(),
                (k, true) => format!("{k} awaiting you"),
                (k, false) => format!("{k} pending"),
            };
            let bold = match (n, steps_only) {
                (_, true) => format!("{p_words}."),
                (n, _) if n_pending == 0 => format!("{n} {} you.", if n == 1 { "thing needs" } else { "things need" }),
                (n, _) => format!("{n} {} you · {p_words}.", if n == 1 { "thing needs" } else { "things need" }),
            };
            let cause = problems.first().or(pending.first()).map_or(String::new(), |i| i.detail.clone());
            let rest = if didnt_run.is_empty() { cause } else { format!("{cause} {didnt_run}") };
            (bold, rest, if fails > 0 { Tone::Danger } else { Tone::Warn })
        };

        let mut fix_now = None;
        let mut copy = None;
        let mut recheck = false;
        let mut toggle = false;
        let copied = self.copied;
        // Passed checks: as the person last chose; until then, open when
        // the window has room for them (a full page, not half of one).
        let chosen_open = ui.data(|d| d.get_temp::<bool>(Id::unique("health-passed-user")));
        let sub = self.health_sub();
        page_ui::scroll("health-page").show_viewport(ui, |ui, _| {
            // The pages' head, then the banner across the page, then the
            // cards in a 1.2 : 1 grid (one column when compact).
            let origin = ui.cursor().min;
            let mut page = page_ui::Page::begin(ui, origin, rect.width(), "Health", &sub);
            page.split(1.2);
            let (x, full) = (page.x, page.w);
            let bw = button_width(ui, "Check again");
            let text_w = full - BANNER_TEXT_X - bw - 20.0 - 20.0;
            let galley = {
                let wrap = egui::text::TextWrapping { max_width: text_w.max(80.0), max_rows: 2, break_anywhere: false, overflow_character: Some('…') };
                let mut job = egui::text::LayoutJob { wrap, ..Default::default() };
                let fmt = |font: egui::FontId| egui::TextFormat { font_id: font, color: pal().ink, line_height: Some(21.0), ..Default::default() };
                job.append(&bold, 0.0, fmt(semibold(T_BODY + 1.0)));
                if !rest.is_empty() {
                    job.append(&format!(" {rest}"), 0.0, fmt(regular(T_BODY + 1.0)));
                }
                ui.painter().layout_job(job)
            };
            let banner_h = (galley.size().y + 32.0).max(68.0);
            let p = ui.painter().clone();

            // The banner.
            let b = Rect::from_min_size(pos2(x, page.below() + 4.0), vec2(full, banner_h));
            let warm = matches!(tone, Tone::Warn | Tone::Danger);
            if warm {
                p.rect(b, CornerRadius::same(R_GROUP), pal().warn_soft, Stroke::new(1.0, pal().warn.gamma_multiply(0.22)), StrokeKind::Inside);
            } else {
                widgets::group(ui, b, R_GROUP);
            }
            // The verdict as a shape on a tile of its own colour.
            let mark_at = pos2(b.left() + 34.0, b.center().y);
            let tile_tone = match tone { Tone::Warn => pal().warn, Tone::Danger => pal().danger, Tone::Ok => pal().ok, Tone::Quiet => pal().ink3 };
            p.rect_filled(Rect::from_center_size(mark_at, vec2(32.0, 32.0)), CornerRadius::same(9), tile_tone.gamma_multiply(0.15));
            match tone {
                Tone::Warn => widgets::status_mark(&p, mark_at, 11.0, widgets::Mark::Warn),
                Tone::Danger => widgets::status_mark(&p, mark_at, 11.0, widgets::Mark::Error),
                Tone::Ok => widgets::status_mark(&p, mark_at, 11.0, widgets::Mark::Ok),
                Tone::Quiet if running && checks.is_none() => {
                    let turn = if motion::reduced() { 0.0 } else { (now * 5.0) as f32 };
                    widgets::draw_refresh(&p, Rect::from_center_size(mark_at, vec2(16.0, 16.0)), pal().ink2, turn);
                    ui.ctx().request_repaint_after_secs(1.0 / 30.0);
                }
                Tone::Quiet => widgets::status_mark(&p, mark_at, 10.0, widgets::Mark::Off),
            }
            let gh = galley.size().y;
            p.galley(pos2(b.left() + BANNER_TEXT_X, b.center().y - gh / 2.0), galley.clone(), pal().ink);
            let br = Rect::from_min_size(pos2(b.right() - 20.0 - bw, b.center().y - 16.0), vec2(bw, 32.0));
            let label = if running { "Checking…" } else { "Check again" };
            if button(ui, br, Id::unique("recheck"), label, Look::Quiet, !running, Some("Run the checks again")).clicked() {
                recheck = true;
            }
            a11y_text(ui, Id::unique("health-summary"), b, Role::Status, &format!("{bold} {rest}"), Some(Live::Polite));
            page.skip_to(b.bottom() + 18.0);

            // ---- the cards: left (Needs you, Pending), right (Connections, Passed) ----
            let mut left: Vec<Card> = vec![];
            let mut right: Vec<Card> = vec![];
            for (title, list) in [("Needs you", &problems), ("Pending", &pending)] {
                if list.is_empty() {
                    continue;
                }
                let inner = page.width_of(0) - 2.0 * page_ui::CARD_PAD;
                let lines = list.iter().map(|i| {
                    let bw = fix_button_w(ui, i);
                    let room = inner - 22.0 - if bw > 0.0 { bw + 16.0 } else { 0.0 };
                    (Line::Need(i), need_height(ui, i, room))
                }).collect();
                left.push(Card { title, extra: None, lines });
            }
            // Nothing to the left: the cards keep to one column, as wide as it.
            let right_col = usize::from(!left.is_empty());
            let inner = page.width_of(right_col) - 2.0 * page_ui::CARD_PAD - 26.0;
            let passed: Vec<(Line, f32)> = works.iter().map(|i| (Line::Passed(i), passed_height(ui, i, inner))).collect();
            // Is there room for all of it? Each card is its label, rows and a gap.
            let card_h = |rows: f32| CARD_LABEL + rows + page_ui::GAP;
            let left_h: f32 = left.iter().map(|c| card_h(c.lines.iter().map(|l| l.1).sum())).sum();
            let conn_h = if peers.is_empty() { 0.0 } else { card_h(peers.len() as f32 * links::ROW_H) };
            let passed_h = if works.is_empty() { 0.0 } else { card_h(ROW + passed.iter().map(|l| l.1).sum::<f32>()) };
            let stacked = !page.two || right_col == 0;
            let need = if stacked { left_h + conn_h + passed_h } else { left_h.max(conn_h + passed_h) };
            let view_bottom = origin.y + rect.height();
            let spare = view_bottom - FOOTER_ROOM - (page.below() + need);
            let open = chosen_open.unwrap_or(self.health_passed_open || spare >= 0.0);
            // Still room: the connections get taller rows.
            let peer_h = if open && spare >= peers.len() as f32 * 12.0 + 24.0 { links::ROW_H + 12.0 } else { links::ROW_H };
            let lines: Vec<(Line, f32)> = (0..peers.len()).map(|k| (Line::Peer(k), peer_h)).collect();
            let mut checks: Vec<(Line, f32)> = vec![];
            if !works.is_empty() {
                checks.push((Line::Toggle(works.len()), ROW));
                if open {
                    checks.extend(passed);
                }
            }
            if !lines.is_empty() {
                right.push(Card { title: "Connections", extra: Some(format!("last {}", rtt::span_words())), lines });
            }
            // The passed checks, a group of their own.
            if !checks.is_empty() {
                right.push(Card { title: "Passed", extra: None, lines: checks });
            }
            let placed = left.iter().map(|c| (0, c)).chain(right.iter().map(|c| (right_col, c)));
            for (col, card) in placed {
                let mut c = page.card(ui, col, card.title);
                if let Some(extra) = &card.extra {
                    p.text(c.label_right() - vec2(2.0, 0.0), Align2::RIGHT_CENTER, extra, regular(T_META), pal().ink3);
                }
                for (n, (line, h)) in card.lines.iter().enumerate() {
                    let row = c.row(ui, *h);
                    match line {
                        Line::Need(i) => {
                            let bw = fix_button_w(ui, i);
                            let text_w = row.width() - 22.0 - if bw > 0.0 { bw + 16.0 } else { 0.0 };
                            widgets::status_mark(&p, pos2(row.left() + 5.0, row.top() + 23.0), 8.0, verdict_mark(i.verdict));
                            let tx = row.left() + 22.0;
                            let mut ty = row.top() + 14.0;
                            if !i.title.is_empty() {
                                p.text(pos2(tx, ty + 8.0), Align2::LEFT_CENTER, &i.title, medium(T_META), pal().ink2);
                                ty += 18.0;
                            }
                            // What is wrong, in ink at 600; what to do, quieter, under it.
                            ty += widgets::para_code(ui, pos2(tx, ty), text_w, &i.detail, semibold(T_BODY), pal().ink);
                            if let Some(f) = i.fix.clone().filter(|_| shows_fix_text(&i.here)) {
                                widgets::para_code(ui, pos2(tx, ty + 4.0), text_w, &f, regular(T_BODY), pal().ink2);
                            }
                            let br = Rect::from_min_size(pos2(row.right() - bw, row.center().y - 15.0), vec2(bw, 30.0));
                            if let Some((label, job)) = &i.here {
                                let resp = button(ui, br, Id::unique(("fix-here", i.k, n, card.title)), label, Look::Primary, !running, None);
                                if matches!(job, Fix::DesktopGuide) {
                                    // The desktop's own dialog: the steps, beside the button.
                                    egui::Popup::menu(&resp).width(320.0).align(egui::RectAlign::BOTTOM_END).show(|ui| {
                                        ui.set_max_width(320.0);
                                        let steps = [
                                            "1. Look for your desktop's sharing window (“Remote desktop” or “Input capture”). It can be behind other windows; check the taskbar.",
                                            "2. Choose Allow (or Share). oneMK only asks for the mouse and keyboard.",
                                            "3. Back here, choose Check again. Dismissed it? Stop and start oneMK to be asked again.",
                                        ];
                                        for t in steps {
                                            ui.add(egui::Label::new(egui::RichText::new(t).font(regular(T_BODY)).color(pal().ink_hover)).wrap());
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
                                    if button(ui, br, Id::unique(("copy-fix", i.k)), word, Look::Primary, true, Some(&format!("Copy the fix for {}: {cmd}", i.title))).clicked() {
                                        copy = Some((i.k, cmd));
                                    }
                                }
                            }
                            let state = if card.title == "Pending" { "is waiting" } else { "needs attention" };
                            let fix = i.fix.as_deref().filter(|_| shows_fix_text(&i.here)).map_or(String::new(), |f| format!(" {}", f.replace('`', "")));
                            let label = if i.title.is_empty() { format!("{}{fix}", i.detail) } else { format!("{} {state}. {}{fix}", i.title, i.detail) };
                            a11y_text(ui, Id::unique(("need", card.title, n)), row, Role::ListItem, &label, None);
                        }
                        Line::Peer(k) => self.connection_row(ui, &peers[*k], row),
                        Line::Toggle(count) => {
                            let id = Id::unique("health-passed");
                            let resp = ui.interact(widgets::hit_rect(row), id, widgets::sense(ui, Sense::click()));
                            let hover = tween(ui.ctx(), id.with("hover"), f32::from(resp.hovered()), 0.1, motion::out_cubic);
                            widgets::status_mark(&p, pos2(row.left() + 4.0, row.center().y), 8.0, widgets::Mark::Ok);
                            let checks_word = if *count == 1 { "check" } else { "checks" };
                            let what = if needs.is_empty() { format!("{count} {checks_word}") } else { format!("{count} other {checks_word}") };
                            p.text(pos2(row.left() + 22.0, row.center().y), Align2::LEFT_CENTER, &what, regular(T_BODY), pal().ink);
                            // "Passed" and a chevron: down to open, up when open.
                            let c = pos2(row.right() - 6.0, row.center().y);
                            let tint = mix(pal().ink2, pal().ink, hover);
                            let dy = if open { 2.0 } else { -2.0 };
                            p.add(egui::Shape::line(vec![c + vec2(-4.0, dy), c + vec2(0.0, -dy), c + vec2(4.0, dy)], Stroke::new(1.5, tint)));
                            p.text(pos2(row.right() - 20.0, row.center().y), Align2::RIGHT_CENTER, "Passed", regular(T_BODY), pal().ink2);
                            widgets::focus_ring(ui, &resp, row.shrink2(vec2(-8.0, -2.0)), 10);
                            widgets::a11y(&resp, Role::Button, &format!("{what} passed. {}", if open { "Hide them" } else { "Show them" }), None);
                            if resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if resp.clicked() {
                                toggle = true;
                            }
                        }
                        Line::Passed(i) => {
                            let tx = row.left() + 22.0;
                            let text_w = row.width() - 26.0;
                            widgets::status_mark(&p, pos2(row.left() + 4.0, row.top() + if i.title.is_empty() { 23.0 } else { 22.0 }), 8.0, widgets::Mark::Ok);
                            if i.title.is_empty() {
                                widgets::para_code(ui, pos2(tx, row.top() + 14.0), text_w, &i.detail, regular(T_BODY), pal().ink);
                            } else {
                                p.text(pos2(tx, row.top() + 22.0), Align2::LEFT_CENTER, &i.title, regular(T_BODY), pal().ink);
                                widgets::para_code(ui, pos2(tx, row.top() + 32.0), text_w, &i.detail, regular(T_META), pal().ink3);
                            }
                            a11y_text(ui, Id::unique(("works", n)), row, Role::ListItem, &if i.title.is_empty() { i.detail.clone() } else { format!("{} passed. {}", i.title, i.detail) }, None);
                        }
                    }
                }
                page.finish(ui, c);
            }
            // When it last checked, and what it runs, at the foot.
            if !sub.is_empty() {
                page.footer(ui, view_bottom, &format!("{sub} · the same checks as `one-mk doctor`"));
            }
            ui.allocate_space(vec2(rect.width(), page.height()));
            if toggle {
                ui.data_mut(|d| d.insert_temp(Id::unique("health-passed-user"), !open));
                self.health_passed_open = !open;
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
            Some(Fix::OpenFirewallSettings) => about::open_link("x-apple.systempreferences:com.apple.Network-Settings.extension?Firewall"),
            Some(Fix::OpenPermissionSettings) => about::open_link("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"),
            _ => {}
        }
    }
}

/// What a card's label takes above it (`page_ui`'s).
const CARD_LABEL: f32 = 30.0;
/// The room a page footer takes under the cards.
const FOOTER_ROOM: f32 = 84.0;

/// Where the banner's words start (after its verdict tile).
const BANNER_TEXT_X: f32 = 64.0;

/// A row of Health or Settings (the handoff's 52).
const ROW: f32 = 52.0;

#[cfg(test)]
mod health_tests {
    use super::*;

    fn check(id: &str) -> Check {
        Check { id: id.into(), title: id.into(), detail: "Not paired; permission not granted".into(), verdict: Verdict::Warn, fix: Some("Pair".into()), action: Some("Pair".into()) }
    }

    #[test]
    fn live_peers_supersede_a_stale_not_paired_check() {
        let status = Status::decode("ok\nname\tDesk\nid\tabcd-ef01\npeer\t1111-2222\tleft\t1\t1\tauto\tStudio PC\n").unwrap();
        let result = current_check(check("pairing"), Some(&status)).unwrap();
        assert_eq!(result.verdict, Verdict::Pass);
        assert_eq!(result.detail, "1 paired computer is saved.");
        assert!(result.fix.is_none() && result.action.is_none());
    }

    #[test]
    fn a_live_permission_request_has_one_authoritative_row() {
        let status = Status::decode("ok\nname\tDesk\nid\tabcd-ef01\ninput\tWaiting for your approval: allow one-mk in the desktop's dialog.\n").unwrap();
        assert!(current_check(check("input"), Some(&status)).is_none());
        assert!(current_check(check("firewall"), Some(&status)).is_some());
    }
}
