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
    Appearance(crate::style::Appearance),
    /// A setting on the Settings page: its category and row.
    Setting(settings_page::Cat, &'static str),
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
    ("Arrange · Where each computer sits", Tab::Arrange),
    ("Arrange · Profiles and Wi-Fi", Tab::Arrange),
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
        // The pages in the bar's order, each with its shortcut.
        let tab = |t: Tab, title: &str| Entry { group: "Go to", title: title.into(), hint: shell::shortcut(t), act: Act::Tab(t) };
        out.push(tab(Tab::Desk, "Desk"));
        out.push(tab(Tab::Arrange, "Arrange computers"));
        out.push(tab(Tab::Wallpaper, "Wallpaper"));
        out.push(tab(Tab::Files, "Clipboard & files"));
        out.push(tab(Tab::Keys, "Keys & mouse"));
        out.push(tab(Tab::Health, "Health"));
        out.push(tab(Tab::Settings, "Settings"));
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
                    out.push(act("Choose a wallpaper video or picture…".into(), "", Act::WallChoose));
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
        for a in crate::style::Appearance::ALL {
            let hint = if a == self.prefs.appearance { "Now" } else { "" };
            out.push(Entry { group: "Settings", title: format!("Appearance: {}", a.label()), hint: hint.into(), act: Act::Appearance(a) });
        }
        // Settings' own rows: each opens its category and lights its row.
        for (title, cat, key) in settings_page::JUMPS {
            out.push(Entry { group: "Settings", title: (*title).into(), hint: format!("Settings · {}", cat.label()), act: Act::Setting(*cat, key) });
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
            Act::Appearance(a) => self.set_appearance(a),
            Act::Setting(cat, key) => self.jump_to_setting(cat, key),
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
        ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("palette-dim"))).rect_filled(screen, 0.0, pal().shadow((140.0 * fade) as u8));

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
            icons::search(&p, pos2(input.left() + 26.0, input.center().y), pal().ink2);
            let field = Rect::from_min_max(pos2(input.left() + 46.0, input.top() + 12.0), pos2(input.right() - 18.0, input.bottom() - 12.0));
            let mut child = ui.new_child(UiBuilder::new().max_rect(field));
            let edit = egui::TextEdit::singleline(&mut self.palette.query)
                .id(Id::unique("palette-query"))
                .font(regular(17.0))
                .text_color(pal().ink)
                .hint_text(egui::RichText::new("Search actions, computers and settings").color(pal().ink3))
                .frame(egui::Frame::NONE)
                .desired_width(field.width());
            let resp = child.add(edit);
            resp.request_focus();
            p.hline(input.x_range(), input.bottom() - 0.5, Stroke::new(1.0, pal().line));
            // The results.
            let list = Rect::from_min_size(pos2(r.left(), input.bottom() + 4.0), vec2(r.width(), list_h));
            if n == 0 {
                p.text(list.center(), Align2::CENTER_CENTER, "Nothing found. Try another word.", regular(T_BODY), pal().ink3);
            }
            let mut child = ui.new_child(UiBuilder::new().max_rect(list));
            child.set_clip_rect(list);
            egui::ScrollArea::vertical().id_salt("palette-list").max_height(list_h).auto_shrink([false, false]).show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (head, k) in &rows {
                    if let Some(g) = head {
                        let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEAD), Sense::hover());
                        ui.painter().text(pos2(hr.left() + 16.0, hr.center().y + 3.0), Align2::LEFT_CENTER, g.to_uppercase(), semibold(T_META - 1.5), pal().ink3);
                        continue;
                    }
                    let (e, lit) = &found[*k];
                    let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                    let row = rr.shrink2(vec2(8.0, 2.0));
                    if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                        self.palette.sel = *k;
                    }
                    // Tabbed onto, a row is the chosen one (its lift and Volt
                    // tile are the focus look), and its ring shows.
                    if resp.gained_focus() {
                        self.palette.sel = *k;
                    }
                    widgets::focus_ring(ui, &resp, row, 10);
                    let chosen = self.palette.sel == *k;
                    if chosen {
                        // The chosen row: a quiet lift, as a system list does; its icon
                        // tile carries the Volt.
                        ui.painter().rect(row, CornerRadius::same(10), pal().accent_soft, Stroke::new(1.0, pal().line), StrokeKind::Inside);
                        if down || up {
                            ui.scroll_to_rect(rr, None);
                        }
                    }
                    let ic = Rect::from_center_size(pos2(row.left() + 22.0, row.center().y), vec2(26.0, 26.0));
                    ui.painter().rect_filled(ic, CornerRadius::same(7), if chosen { pal().accent } else { pal().hover });
                    let icon = match e.act {
                        Act::Tab(Tab::Files) => Icon::Clipboard,
                        Act::Tab(Tab::Keys) => Icon::Keys,
                        Act::Tab(Tab::Settings) | Act::Appearance(_) | Act::Setting(..) => Icon::Settings,
                        Act::Tab(Tab::Health) | Act::Recheck => Icon::Health,
                        Act::Tab(Tab::Arrange) => Icon::Arrange,
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
                    icons::draw(ui.painter(), icon, ic.shrink(5.5), if chosen { pal().on_accent } else { pal().ink });
                    // The title with what was typed lit up.
                    let mut job = egui::text::LayoutJob::default();
                    let mut at = 0;
                    let mut ranges = lit.clone();
                    ranges.sort_by_key(|r| r.start);
                    for range in ranges {
                        if range.start < at {
                            continue;
                        }
                        // Only the typed characters stand out: in full ink and
                        // semibold, the rest of the title a step quieter.
                        job.append(&e.title[at..range.start], 0.0, egui::TextFormat::simple(regular(14.0), pal().ink_hover));
                        job.append(&e.title[range.clone()], 0.0, egui::TextFormat::simple(semibold(14.0), pal().ink));
                        at = range.end;
                    }
                    job.append(&e.title[at..], 0.0, egui::TextFormat::simple(regular(14.0), pal().ink_hover));
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(ic.right() + 12.0, row.center().y - g.size().y / 2.0), g, pal().ink);
                    if !e.hint.is_empty() {
                        ui.painter().text(pos2(row.right() - 12.0, row.center().y), Align2::RIGHT_CENTER, &e.hint, regular(T_META), pal().ink3);
                    } else if chosen {
                        ui.painter().text(pos2(row.right() - 12.0, row.center().y), Align2::RIGHT_CENTER, "↵", regular(T_BODY), pal().ink2);
                    }
                    widgets::a11y(&resp, Role::Button, &e.title, Some(chosen));
                    if resp.clicked() {
                        run = Some(e.act.clone());
                    }
                }
            });
            // What the keys do.
            let foot = Rect::from_min_max(pos2(r.left(), r.bottom() - 40.0), r.max);
            p.hline(foot.x_range(), foot.top() + 0.5, Stroke::new(1.0, pal().line));
            p.text(pos2(foot.left() + 16.0, foot.center().y), Align2::LEFT_CENTER, "oneMK", regular(T_META), pal().ink3);
            let mut x = foot.right() - 16.0;
            for (word, key) in [("Close", "esc"), ("Move", "↑↓"), ("Open", "↵")] {
                let kr = widgets::kbd(&p, pos2(x, foot.center().y), Align2::RIGHT_CENTER, key);
                let ww = text_width(ui, word, regular(T_META));
                p.text(pos2(kr.left() - 6.0, foot.center().y), Align2::RIGHT_CENTER, word, regular(T_META), pal().ink3);
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
    fn settings_titles_come_from_settings_so_each_lands_on_its_row() {
        // Settings' own rows are listed by `settings_page::JUMPS` (each opens
        // its category and lights its row); this list holds other pages'.
        assert!(SETTINGS.iter().all(|(_, tab)| *tab != Tab::Settings));
        // A typed query finds a setting by any word of its title, as the
        // palette does, and the row it stands for is in the category it says.
        for (query, cat, key) in [("wait at", settings_page::Cat::Edges, "edge-dwell"), ("pointer wrap", settings_page::Cat::Pointer, "pointer-wrap"),
            ("look for updates", settings_page::Cat::General, "update-check"), ("stop when idle", settings_page::Cat::Typing, "broadcast-idle"),
            ("ctrl and cmd", settings_page::Cat::Typing, "swaps"), ("run commands", settings_page::Cat::Commands, "hook-0"),
            ("gaming", settings_page::Cat::Games, "gaming"), ("keyboard shortcuts", settings_page::Cat::Shortcuts, "hotkeys"),
            ("lock every screen with", settings_page::Cat::Power, "lock-sync")] {
            let found: Vec<_> = settings_page::JUMPS.iter().filter(|(title, _, _)| matches(title, query).is_some()).collect();
            assert!(found.iter().any(|(_, c, k)| *c == cat && *k == key), "{query}: {found:?}");
        }
    }

    #[test]
    fn a_title_that_starts_with_the_query_comes_first() {
        assert!(score("Lock every screen", "lock") < score("Keep the pointer here and lock", "lock"));
        assert!(score("Clipboard history", "history") < score("Clipboardhistory", "history"));
    }
}
