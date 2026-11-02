//! The command palette (⌘K, Ctrl+K elsewhere): type a few letters and go
//! anywhere or do anything — switch to a computer, send it the clipboard,
//! lock every screen, open a page, find a setting. Up and Down choose,
//! Return does it, Esc closes. It is the one piece of real glass in the
//! window, over the window dimmed.

use super::*;
use crate::icons::{self, Icon};

#[derive(Default)]
pub(super) struct Palette {
    pub open: bool,
    pub query: String,
    sel: usize,
    /// The query the choice was made for: a new query chooses the first.
    sel_for: String,
    opened: f64,
}

#[derive(Clone)]
enum Act {
    Tab(Tab),
    Pair,
    LockAll,
    Broadcast(bool),
    Switch(String, String),
    Here,
    Clip(bool, String, String),
    KeepHere(bool),
    Received,
    Start,
    Stop,
    Recheck,
    Update,
    WallChoose,
    WallPause(bool),
    WallOff,
}

struct Entry {
    group: &'static str,
    title: String,
    hint: String,
    act: Act,
}

/// Settings the palette finds, and the page each is on.
const SETTINGS: &[(&str, Tab)] = &[
    ("Wallpaper · Fill, fit or stretch", Tab::Wallpaper),
    ("Wallpaper · Pause on battery", Tab::Wallpaper),
    ("Wallpaper · Play on a computer", Tab::Wallpaper),
    ("Crossing edges · Don't cross near corners", Tab::Advanced),
    ("Crossing edges · Wait at the edge", Tab::Advanced),
    ("Crossing edges · Hold a key to cross", Tab::Advanced),
    ("Crossing edges · Stay in fullscreen windows", Tab::Advanced),
    ("Power · Keep computers I control awake", Tab::Advanced),
    ("Power · Sleep displays together", Tab::Advanced),
    ("Power · Lock all when this one locks", Tab::Advanced),
    ("Games · Gaming mode", Tab::Advanced),
    ("Keyboard shortcuts", Tab::Advanced),
    ("Ctrl and Cmd · Swap them between a Mac and a PC", Tab::Advanced),
    ("Run commands when the pointer arrives or leaves", Tab::Advanced),
    ("Typing on the other computer · Letters across layouts", Tab::Advanced),
    ("Pointer wrap", Tab::Advanced),
    ("Learn computer positions", Tab::Advanced),
    ("Draw a pointer on computers with no mouse", Tab::Advanced),
    ("Updates · Look for updates", Tab::Advanced),
    ("Updates · Install them without asking", Tab::Advanced),
    ("Clipboard · Text, rich text and images", Tab::Files),
    ("Clipboard · Others can get this clipboard", Tab::Files),
    ("Clipboard · Skip sensitive copies", Tab::Files),
    ("Clipboard history", Tab::Files),
    ("Largest clipboard", Tab::Files),
    ("Files · Drop files at screen edges", Tab::Files),
    ("Files · Notify when files arrive", Tab::Files),
    ("Keys · Change what a key types", Tab::Keys),
    ("Keys · Caps Lock as Esc or Ctrl", Tab::Keys),
    ("Mouse · Buttons", Tab::Keys),
    ("Mouse · Pointer speed", Tab::Keys),
    ("Mouse · Scrolling", Tab::Keys),
    ("Mouse · Gestures", Tab::Keys),
    ("Mouse · Autoscroll", Tab::Keys),
    ("Layout · Where each computer sits", Tab::Layout),
    ("Layout · Profiles and Wi-Fi", Tab::Layout),
    ("Start at login", Tab::Desk),
];

/// Where `query`'s words are found in `text` (every word must be), as
/// byte ranges to light; `None` when one is missing.
fn matches(text: &str, query: &str) -> Option<Vec<std::ops::Range<usize>>> {
    let lower = text.to_lowercase();
    // Lowercasing can change byte lengths; only light up when it doesn't.
    let same = lower.len() == text.len();
    let mut out = vec![];
    for word in query.split_whitespace() {
        let w = word.to_lowercase();
        let at = lower.find(&w)?;
        if same {
            out.push(at..at + w.len());
        }
    }
    Some(out)
}

/// How well a match scores: a word that starts the title first, then one
/// that starts a word, then anywhere.
fn score(text: &str, query: &str) -> u32 {
    let lower = text.to_lowercase();
    let q = query.trim().to_lowercase();
    if lower.starts_with(&q) {
        0
    } else if lower.contains(&format!(" {q}")) {
        1
    } else {
        2
    }
}

impl App {
    fn palette_entries(&self) -> Vec<Entry> {
        let mut out = vec![];
        let k = shell::cmd();
        let tab = |t: Tab, title: &str, hint: String| Entry { group: "Go to", title: title.into(), hint, act: Act::Tab(t) };
        out.push(tab(Tab::Desk, "Desk", format!("{k}1")));
        out.push(tab(Tab::Layout, "Arrange computers", "Desk".into()));
        out.push(tab(Tab::Wallpaper, "Wallpaper", format!("{k}2")));
        out.push(tab(Tab::Files, "Clipboard & files", format!("{k}3")));
        out.push(tab(Tab::Keys, "Keys & mouse", format!("{k}4")));
        out.push(tab(Tab::Advanced, "Settings", format!("{k},")));
        out.push(tab(Tab::Health, "Health", String::new()));
        let act = |title: String, hint: &str, act: Act| Entry { group: "Actions", title, hint: hint.into(), act };
        match self.status.as_ref() {
            Some(None) => out.push(act("Start oneMK".into(), "", Act::Start)),
            Some(Some(s)) => {
                out.push(act("Add a computer".into(), "", Act::Pair));
                if s.peers.iter().any(|p| p.online) {
                    out.push(act("Lock every screen".into(), "", Act::LockAll));
                    let on = self.broadcasting();
                    out.push(act(if on { "Stop typing on every computer".into() } else { "Type on every computer".into() }, "", Act::Broadcast(!on)));
                    out.push(act("Back to this computer".into(), "", Act::Here));
                    let kept = s.pointer_locked;
                    out.push(act(if kept { "Let the pointer cross again".into() } else { "Keep the pointer here".into() }, "", Act::KeepHere(!kept)));
                }
                if let Some(w) = s.wallpaper.as_ref().filter(|w| w.available) {
                    out.push(act("Choose a wallpaper video…".into(), "", Act::WallChoose));
                    if let Some(plan) = &w.plan {
                        out.push(act(if plan.paused { "Play the wallpaper".into() } else { "Pause the wallpaper".into() }, "", Act::WallPause(!plan.paused)));
                        out.push(act("Turn the wallpaper off".into(), "", Act::WallOff));
                    }
                }
                out.push(act("Open received files".into(), "", Act::Received));
                out.push(act("Check health again".into(), "", Act::Recheck));
                out.push(act("Stop oneMK".into(), "", Act::Stop));
                for p in s.peers.iter().filter(|p| p.online) {
                    let c = |title: String, act: Act| Entry { group: "Computers", title, hint: String::new(), act };
                    out.push(c(format!("Switch to {}", p.name), Act::Switch(p.id.clone(), p.name.clone())));
                    out.push(c(format!("Send the clipboard to {}", p.name), Act::Clip(true, p.id.clone(), p.name.clone())));
                    out.push(c(format!("Get the clipboard from {}", p.name), Act::Clip(false, p.id.clone(), p.name.clone())));
                }
            }
            None => {}
        }
        if let Some(v) = &self.update_available {
            out.push(act(format!("Update oneMK to {v}"), "", Act::Update));
        }
        for (title, t) in SETTINGS {
            let page = TABS.iter().find(|x| x.0 == *t).map_or("", |x| x.1);
            out.push(Entry { group: "Settings", title: (*title).into(), hint: page.into(), act: Act::Tab(*t) });
        }
        out
    }

    pub(super) fn open_palette(&mut self, now: f64) {
        self.palette.open = true;
        self.palette.query.clear();
        self.palette.sel = 0;
        self.palette.opened = now;
    }

    fn run(&mut self, act: Act, now: f64) {
        self.palette.open = false;
        match act {
            Act::Tab(t) => self.set_tab(t),
            Act::Pair => self.start_pairing(now),
            Act::LockAll => {
                self.busy.push(Job::LockAll);
                self.link.lock_all();
            }
            Act::Broadcast(on) => self.ask_broadcast(on),
            Act::Switch(id, name) => self.link.switch(&id, &name),
            Act::Here => self.link.switch("here", "this computer"),
            Act::Clip(send, id, name) => self.link.clip(send, &id, &name),
            Act::KeepHere(on) => self.link.lock_pointer(on),
            Act::Received => self.link.open_downloads(),
            Act::Start => {
                self.busy.push(Job::Start);
                self.link.start();
            }
            Act::Stop => {
                self.busy.push(Job::Stop);
                self.link.stop();
            }
            Act::Recheck => {
                self.set_tab(Tab::Health);
                self.busy.push(Job::Doctor);
                self.link.doctor();
            }
            Act::Update => self.begin_update(),
            Act::WallChoose => {
                self.set_tab(Tab::Wallpaper);
                let ctx = self.ctx.clone();
                self.choose_video(&ctx);
            }
            Act::WallPause(paused) => self.wallpaper_command(one_mk_wallpaper::Command::Pause { paused }),
            Act::WallOff => self.wallpaper_command(one_mk_wallpaper::Command::Off),
        }
    }

    pub(super) fn palette(&mut self, ctx: &egui::Context, now: f64) {
        if !self.palette.open {
            return;
        }
        let screen = ctx.content_rect();
        // The window dimmed under it; a click there closes it.
        let fade = motion::progress(now, self.palette.opened, motion::fade(0.14), motion::out_cubic);
        if fade < 1.0 {
            ctx.request_repaint();
        }
        ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("palette-dim"))).rect_filled(screen, 0.0, Color32::from_black_alpha((140.0 * fade) as u8));

        let q = self.palette.query.trim().to_owned();
        let mut found: Vec<(Entry, Vec<std::ops::Range<usize>>)> = self.palette_entries().into_iter().filter_map(|e| {
            if q.is_empty() {
                // Nothing typed: the pages and the actions, not every setting.
                return (e.group != "Settings").then_some((e, vec![]));
            }
            let m = matches(&e.title, &q)?;
            Some((e, m))
        }).collect();
        if !q.is_empty() {
            let order = |g: &str| ["Actions", "Computers", "Go to", "Settings"].iter().position(|x| *x == g).unwrap_or(9);
            found.sort_by_key(|(e, _)| (order(e.group), score(&e.title, &q)));
        }
        if self.palette.sel_for != q {
            self.palette.sel = 0;
            self.palette.sel_for = q.clone();
        }
        let n = found.len();
        let (up, down, enter, esc) = ctx.input_mut(|i| (
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Escape),
        ));
        if esc {
            self.palette.open = false;
            return;
        }
        if n > 0 {
            if down { self.palette.sel = (self.palette.sel + 1) % n; }
            if up { self.palette.sel = (self.palette.sel + n - 1) % n; }
        }
        self.palette.sel = self.palette.sel.min(n.saturating_sub(1));
        if enter && let Some((e, _)) = found.get(self.palette.sel) {
            let act = e.act.clone();
            self.run(act, now);
            return;
        }

        // Rows: a heading where the group changes, then the entries.
        const ROW: f32 = 40.0;
        const HEAD: f32 = 26.0;
        const MAX_H: f32 = 380.0;
        let mut rows: Vec<(Option<&'static str>, usize)> = vec![];
        let mut last = "";
        for (k, (e, _)) in found.iter().enumerate() {
            if e.group != last {
                rows.push((Some(e.group), k));
                last = e.group;
            }
            rows.push((None, k));
        }
        let list_h: f32 = rows.iter().map(|(h, _)| if h.is_some() { HEAD } else { ROW }).sum::<f32>().min(MAX_H).max(if n == 0 { 64.0 } else { 0.0 });
        let w = (screen.width() - 80.0).min(620.0);
        let h = 56.0 + list_h + 8.0 + 40.0;
        let rect = Rect::from_min_size(pos2(screen.center().x - w / 2.0, screen.top() + (screen.height() * 0.16).max(60.0) + 6.0 * (1.0 - fade)), vec2(w, h));
        let clicked_out = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p)));
        if clicked_out && now - self.palette.opened > 0.1 {
            self.palette.open = false;
            return;
        }
        let mut run = None;
        egui::Area::new(Id::unique("palette")).order(egui::Order::Tooltip).fixed_pos(rect.min).show(ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
            for s in widgets::glass_shapes_as(ui.ctx(), Id::unique("palette-glass").value(), r, 18.0, widgets::PALETTE_GLASS) {
                ui.painter().add(s);
            }
            let p = ui.painter().clone();
            // The query.
            let input = Rect::from_min_size(r.min, vec2(r.width(), 56.0));
            icons::search(&p, pos2(input.left() + 26.0, input.center().y), INK2);
            let field = Rect::from_min_max(pos2(input.left() + 46.0, input.top() + 12.0), pos2(input.right() - 18.0, input.bottom() - 12.0));
            let mut child = ui.new_child(UiBuilder::new().max_rect(field));
            let edit = egui::TextEdit::singleline(&mut self.palette.query)
                .id(Id::unique("palette-query"))
                .font(regular(17.0))
                .text_color(INK)
                .hint_text(egui::RichText::new("Search actions, computers and settings").color(INK3))
                .frame(egui::Frame::NONE)
                .desired_width(field.width());
            let resp = child.add(edit);
            resp.request_focus();
            p.hline(input.x_range(), input.bottom() - 0.5, Stroke::new(1.0, HAIRLINE));
            // The results.
            let list = Rect::from_min_size(pos2(r.left(), input.bottom() + 4.0), vec2(r.width(), list_h));
            if n == 0 {
                p.text(list.center(), Align2::CENTER_CENTER, "Nothing found. Try another word.", regular(T_BODY), INK3);
            }
            let mut child = ui.new_child(UiBuilder::new().max_rect(list));
            child.set_clip_rect(list);
            egui::ScrollArea::vertical().id_salt("palette-list").max_height(list_h).auto_shrink([false, false]).show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (head, k) in &rows {
                    if let Some(g) = head {
                        let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEAD), Sense::hover());
                        ui.painter().text(pos2(hr.left() + 16.0, hr.center().y + 3.0), Align2::LEFT_CENTER, g.to_uppercase(), semibold(T_META - 1.5), INK3);
                        continue;
                    }
                    let (e, lit) = &found[*k];
                    let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                    let row = rr.shrink2(vec2(8.0, 2.0));
                    if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                        self.palette.sel = *k;
                    }
                    let chosen = self.palette.sel == *k;
                    if chosen {
                        ui.painter().rect(row, CornerRadius::same(10), SIGNAL.gamma_multiply(0.12), Stroke::new(1.0, SIGNAL.gamma_multiply(0.35)), StrokeKind::Inside);
                        if down || up {
                            ui.scroll_to_rect(rr, None);
                        }
                    }
                    let ic = Rect::from_center_size(pos2(row.left() + 22.0, row.center().y), vec2(26.0, 26.0));
                    ui.painter().rect_filled(ic, CornerRadius::same(7), if chosen { SIGNAL } else { Color32::from_white_alpha(18) });
                    let icon = match e.act {
                        Act::Tab(Tab::Files) => Icon::Clipboard,
                        Act::Tab(Tab::Keys) => Icon::Keys,
                        Act::Tab(Tab::Advanced) => Icon::Advanced,
                        Act::Tab(Tab::Health) | Act::Recheck => Icon::Health,
                        Act::Tab(Tab::Layout) => Icon::Layout,
                        Act::Tab(Tab::Wallpaper) | Act::WallChoose | Act::WallPause(_) | Act::WallOff => Icon::Wallpaper,
                        Act::Tab(_) => Icon::Desk,
                        Act::LockAll | Act::KeepHere(_) => Icon::Lock,
                        Act::Broadcast(_) => Icon::Broadcast,
                        Act::Clip(true, ..) => Icon::Send,
                        Act::Clip(false, ..) => Icon::Get,
                        Act::Switch(..) | Act::Here => Icon::Move,
                        Act::Pair => Icon::Plus,
                        Act::Start | Act::Stop => Icon::Power,
                        Act::Update => Icon::Update,
                        Act::Received => Icon::Clipboard,
                    };
                    icons::draw(ui.painter(), icon, ic.shrink(5.5), if chosen { SIGNAL_INK } else { INK });
                    // The title with what was typed lit up.
                    let mut job = egui::text::LayoutJob::default();
                    let mut at = 0;
                    let mut ranges = lit.clone();
                    ranges.sort_by_key(|r| r.start);
                    for range in ranges {
                        if range.start < at {
                            continue;
                        }
                        job.append(&e.title[at..range.start], 0.0, egui::TextFormat::simple(regular(14.0), INK));
                        job.append(&e.title[range.clone()], 0.0, egui::TextFormat::simple(semibold(14.0), SIGNAL));
                        at = range.end;
                    }
                    job.append(&e.title[at..], 0.0, egui::TextFormat::simple(regular(14.0), INK));
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(ic.right() + 12.0, row.center().y - g.size().y / 2.0), g, INK);
                    if !e.hint.is_empty() {
                        ui.painter().text(pos2(row.right() - 12.0, row.center().y), Align2::RIGHT_CENTER, &e.hint, regular(T_META), INK3);
                    } else if chosen {
                        ui.painter().text(pos2(row.right() - 12.0, row.center().y), Align2::RIGHT_CENTER, "↵", regular(T_BODY), INK2);
                    }
                    widgets::a11y(&resp, Role::Button, &e.title, Some(chosen));
                    if resp.clicked() {
                        run = Some(e.act.clone());
                    }
                }
            });
            // What the keys do.
            let foot = Rect::from_min_max(pos2(r.left(), r.bottom() - 40.0), r.max);
            p.hline(foot.x_range(), foot.top() + 0.5, Stroke::new(1.0, HAIRLINE));
            p.text(pos2(foot.left() + 16.0, foot.center().y), Align2::LEFT_CENTER, "oneMK", regular(T_META), INK3);
            let mut x = foot.right() - 16.0;
            for (word, key) in [("Close", "esc"), ("Move", "↑↓"), ("Open", "↵")] {
                let kw = text_width(ui, key, regular(T_META - 1.0)) + 12.0;
                let kr = Rect::from_min_size(pos2(x - kw, foot.center().y - 10.0), vec2(kw, 20.0));
                p.rect(kr, CornerRadius::same(5), Color32::from_white_alpha(14), Stroke::new(0.5, Color32::from_white_alpha(40)), StrokeKind::Inside);
                p.text(kr.center(), Align2::CENTER_CENTER, key, regular(T_META - 1.0), INK2);
                let ww = text_width(ui, word, regular(T_META));
                p.text(pos2(kr.left() - 6.0, foot.center().y), Align2::RIGHT_CENTER, word, regular(T_META), INK3);
                x = kr.left() - 6.0 - ww - 14.0;
            }
        });
        if let Some(act) = run {
            self.run(act, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_must_be_found_and_is_lit() {
        assert_eq!(matches("Send the clipboard to Studio PC", "clip stu"), Some(vec![9..13, 22..25]));
        assert!(matches("Lock every screen", "lock wall").is_none());
        // Case doesn't matter.
        assert!(matches("Keyboard shortcuts", "KEYB").is_some());
    }

    #[test]
    fn a_title_that_starts_with_the_query_comes_first() {
        assert!(score("Lock every screen", "lock") < score("Keep the pointer here and lock", "lock"));
        assert!(score("Clipboard history", "history") < score("Clipboardhistory", "history"));
    }
}
