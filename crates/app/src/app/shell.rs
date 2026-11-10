//! The window's frame: the top bar across the whole window (60 points), and
//! the page under it. The bar holds what is true on every page: the logo,
//! where to go (each page with its shortcut), an update when one waits,
//! whether everything works (a status line that opens Health), Search (the
//! command palette), and the one action that matters most right now.
//! On macOS the bar is the title bar: its content starts after the window
//! buttons, and its empty part moves the window.

use super::*;
use crate::chrome;
use crate::icons;
use egui::FontId;

/// The top bar's height.
pub(super) const BAR_H: f32 = 60.0;
/// The page's title row under the bar (until each page's own step gives
/// it its new header).
pub(super) const HEAD_H: f32 = 44.0;
/// Narrower than this the window is compact: short page names, "Pair".
pub(super) const COMPACT_BELOW: f32 = 1000.0;
/// From this wide the status line shows in the bar.
const STATUS_FROM: f32 = 1280.0;
/// From this wide Search shows its word.
const SEARCH_WORD_FROM: f32 = 1100.0;
/// Where the bar's content starts on macOS, after the window buttons.
pub(super) const MAC_INSET: f32 = 78.0;
const PAD_X: f32 = 20.0;
const GAP: f32 = 18.0;
const RIGHT_GAP: f32 = 10.0;
const TAB_GAP: f32 = 2.0;
const CONTROL_H: f32 = 34.0;
const SYMBOL: f32 = 24.0;
const PILL_H: f32 = 26.0;

/// The pages in the bar, in order, each with its shortcut key (with ⌘ or
/// Ctrl): 1 to 7 in this order; Settings also has the usual ⌘, (Ctrl+,).
pub(super) const NAV: [(Tab, &str); 7] = [
    (Tab::Desk, "1"),
    (Tab::Arrange, "2"),
    (Tab::Wallpaper, "3"),
    (Tab::Files, "4"),
    (Tab::Keys, "5"),
    (Tab::Health, "6"),
    (Tab::Settings, "7"),
];

/// Whether a page draws its own title (22, 600, with its sentence under
/// it) in place of the compact title row: the pages of cards, and Keys &
/// mouse's header row. The Desk has no title: its panel says it.
pub(super) fn own_head(tab: Tab) -> bool {
    matches!(tab, Tab::Desk | Tab::Arrange | Tab::Wallpaper | Tab::Files | Tab::Keys)
}

/// The shortcut's modifier as this system writes it.
pub(super) fn cmd() -> &'static str {
    if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" }
}

/// The shortcut shown for a page: ⌘1 (Ctrl+1); Settings ⌘, (Ctrl+,).
pub(super) fn shortcut(tab: Tab) -> String {
    let key = if tab == Tab::Settings { "," } else { NAV.iter().find(|n| n.0 == tab).map_or("", |n| n.1) };
    format!("{}{key}", cmd())
}

/// The page a shortcut key opens (with ⌘ or Ctrl).
pub(super) fn tab_for_key(key: Key) -> Option<Tab> {
    let n = match key {
        Key::Comma => return Some(Tab::Settings),
        Key::Num1 => 0,
        Key::Num2 => 1,
        Key::Num3 => 2,
        Key::Num4 => 3,
        Key::Num5 => 4,
        Key::Num6 => 5,
        Key::Num7 => 6,
        _ => return None,
    };
    Some(NAV[n].0)
}

/// Every key that opens a page.
pub(super) const PAGE_KEYS: [Key; 8] = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Comma];

/// The page's name in the bar: shorter in a compact window (Files, Keys).
pub(super) fn nav_label(tab: Tab, short: bool) -> &'static str {
    TABS.iter().find(|t| t.0 == tab).map_or("", |t| if short { t.2 } else { t.1 })
}

/// Search's key chip: ⌘K on a Mac, Ctrl K elsewhere.
pub(super) fn search_chip() -> &'static str {
    if cfg!(target_os = "macos") { "⌘K" } else { "Ctrl K" }
}

/// The action's label, shorter step by step: 0 as it is, 1 shorter, 2 the
/// shortest (a compact window's "Pair").
pub(super) fn action_label(label: &str, level: u8) -> &str {
    match (label, level) {
        (_, 0) => label,
        ("Pair another computer", 1) => "Pair another",
        ("Pair a computer" | "Pair another computer", _) => "Pair",
        ("Use keyboard to stop", _) => "Keyboard stop",
        _ => label,
    }
}

/// What the bar shows at a given width: which words, how roomy.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Bar {
    pub short_nav: bool,
    pub tab_pad: f32,
    pub wordmark: bool,
    pub status: bool,
    pub search_word: bool,
    pub action_level: u8,
    /// The update pill says only "Update" (its full words are its name).
    pub pill_short: bool,
    /// Search shows its key chip (the last thing to go, in the tightest
    /// bar: its tooltip still names the shortcut).
    pub search_chip: bool,
}

/// The update pill's words as the bar shows them.
fn pill_text<'a>(bar: &Bar, text: &'a str) -> &'a str {
    if bar.pill_short { "Update" } else { text }
}

/// The words the bar carries this frame.
pub(super) struct BarText<'a> {
    pub action: &'a str,
    pub status: Option<&'a str>,
    pub pill: Option<&'a str>,
}

/// How a piece of the bar's text measures (a closure, so tests measure with
/// the window's real fonts).
pub(super) type Measure<'a> = &'a dyn Fn(&str, &FontId) -> f32;

fn wordmark_w(m: Measure) -> f32 {
    m("oneMK", &wordmark(T_WORDMARK)) + WORDMARK_TRACK * 4.0
}

/// The wordmark's tracking: −0.04 em.
const WORDMARK_TRACK: f32 = -0.04 * T_WORDMARK;

fn tab_w(m: Measure, tab: Tab, bar: &Bar) -> f32 {
    m(nav_label(tab, bar.short_nav), &semibold(T_BODY)) + 2.0 * bar.tab_pad
}

fn pill_w(m: Measure, text: &str) -> f32 {
    m(text, &medium(T_META)) + 22.0
}

fn status_w(m: Measure, text: &str) -> f32 {
    7.0 + 7.0 + m(text, &regular(T_META)) + 4.0
}

fn search_w(m: Measure, word: bool, chip: bool) -> f32 {
    let chip = if chip { 8.0 + m(search_chip(), &mono(T_CHIP)) + 12.0 } else { 0.0 };
    10.0 + 15.0 + if word { 8.0 + m("Search", &regular(T_BODY)) } else { 0.0 } + chip + if chip > 0.0 || word { 8.0 } else { 10.0 }
}

fn action_w(m: Measure, label: &str) -> f32 {
    m(label, &semibold(T_BODY)) + 28.0
}

/// The width the bar needs laid out as `bar`, with `inset` at its left.
pub(super) fn bar_need(bar: &Bar, t: &BarText, inset: f32, m: Measure) -> f32 {
    let logo = SYMBOL + if bar.wordmark { 9.0 + wordmark_w(m) } else { 0.0 };
    let nav = NAV.iter().map(|(tab, _)| tab_w(m, *tab, bar)).sum::<f32>() + TAB_GAP * (NAV.len() - 1) as f32;
    let mut right = vec![search_w(m, bar.search_word, bar.search_chip), action_w(m, action_label(t.action, bar.action_level))];
    if let Some(p) = t.pill { right.push(pill_w(m, pill_text(bar, p))); }
    if let (true, Some(s)) = (bar.status, t.status) { right.push(status_w(m, s)); }
    let right = right.iter().sum::<f32>() + RIGHT_GAP * (right.len() - 1) as f32;
    PAD_X + inset.max(PAD_X) + logo + GAP + nav + GAP + right
}

/// The bar for a window `width` wide: the handoff's breakpoints first, then
/// (when the words are longer than the mock's) shorter forms, one step at a
/// time, until it fits: nothing wraps or clips, down to 820 wide.
pub(super) fn plan_bar(width: f32, t: &BarText, inset: f32, m: Measure) -> Bar {
    let compact = width < COMPACT_BELOW;
    let mut bar = Bar {
        short_nav: compact,
        tab_pad: 12.0,
        wordmark: true,
        status: width >= STATUS_FROM && t.status.is_some(),
        search_word: width >= SEARCH_WORD_FROM,
        action_level: if compact { 2 } else { 0 },
        pill_short: false,
        search_chip: true,
    };
    let steps: [fn(&mut Bar); 10] = [
        |b| b.status = false,
        |b| b.action_level = b.action_level.max(1),
        |b| b.search_word = false,
        |b| b.short_nav = true,
        |b| b.tab_pad = 8.0,
        |b| b.action_level = 2,
        |b| b.pill_short = true,
        |b| b.tab_pad = 6.0,
        |b| b.search_chip = false,
        |b| b.wordmark = false,
    ];
    for step in steps {
        if bar_need(&bar, t, inset, m) <= width {
            break;
        }
        step(&mut bar);
    }
    bar
}

/// A computer in the Choose computer menu: its name, then whether it is
/// connected, how fast it answers, where the pointer is, and the choice.
fn computer_menu_label(name: &str, id: &str, online: bool, chosen: bool, here: bool, value: &str) -> String {
    let state = if id == "me" { "This computer" } else if online { "Connected" } else { "Offline" };
    let mut detail = vec![state];
    if !value.is_empty() { detail.push(value); }
    if here { detail.push("Pointer here"); }
    if chosen { detail.push("Selected"); }
    format!("{name}\n{}", detail.join(" · "))
}

/// A page's tab in the bar: its fill, its name, its focus ring.
struct TabLook {
    chosen: bool,
    enabled: bool,
    dot: bool,
}

/// Whether `id` was focused from the keyboard (its ring shows).
fn ring(ui: &Ui, r: &egui::Response, rect: Rect, radius: u8) {
    widgets::focus_ring(ui, r, rect, radius);
}

impl App {
    /// Where the pointer is now: this computer ("me") or a paired one's id.
    pub(super) fn pointer_on(&self) -> Option<String> {
        let s = self.running()?;
        Some(match &s.controlling {
            Some(name) => s.peers.iter().find(|p| &p.name == name).map_or("me".to_owned(), |p| p.id.clone()),
            None => "me".to_owned(),
        })
    }

    /// Something on Health needs attention.
    pub(super) fn health_trouble(&self) -> bool {
        self.running().is_some_and(|s| s.input_lines().iter().any(|line| line.severity == one_mk_ctl::status::Severity::Problem))
            || self.health_checks().is_some_and(|checks| checks.iter().any(|check| matches!(check.verdict, crate::view::Verdict::Fail | crate::view::Verdict::Warn)))
    }

    fn health_tone(&self) -> Color32 {
        if self.running().is_some_and(|status| status.input_lines().iter().any(|line| line.severity == one_mk_ctl::status::Severity::Problem)) || self.health_checks().is_some_and(|checks| checks.iter().any(|check| check.verdict == crate::view::Verdict::Fail)) { pal().danger } else { pal().warn }
    }

    /// Whether everything works, in a word: its mark's colour and the words.
    pub(super) fn status_line(&self) -> (Color32, String) {
        let trouble = self.health_trouble();
        match self.status.as_ref() {
            None => (pal().ink3, "Looking for oneMK…".to_owned()),
            Some(None) => (pal().ink3, "Sharing is off".to_owned()),
            Some(Some(_)) if trouble => (self.health_tone(), "On · check Health".to_owned()),
            // Only "all good" when a computer is actually connected.
            Some(Some(s)) if s.peers.is_empty() => (pal().ink3, "On · not paired".to_owned()),
            Some(Some(s)) if !s.peers.iter().any(|p| p.online) => (pal().ink3, "On · none connected".to_owned()),
            Some(Some(s)) if s.peers.iter().any(|peer| !peer.online) => (pal().warn, "On · some offline".to_owned()),
            Some(Some(_)) => (pal().ok, "On · connected".to_owned()),
        }
    }

    /// The update pill's words, when an update waits (or is under way).
    fn update_pill(&self) -> Option<&'static str> {
        use super::updates::UpdateBox;
        Some(match self.update_box()? {
            UpdateBox::Ready | UpdateBox::MoveFirst => "Update ready",
            UpdateBox::Working { .. } => "Updating…",
            UpdateBox::Failed { .. } => "Update didn't finish",
        })
    }

    /// The top-bar action, done.
    pub(super) fn do_action(&mut self, job: Option<Job>, now: f64) {
        match job {
            Some(Job::Start) => {
                self.busy.push(Job::Start);
                self.link.start();
            }
            Some(Job::TakeBack) => {
                self.busy.push(Job::TakeBack);
                self.link.take_back();
            }
            Some(Job::Broadcast(on)) => self.ask_broadcast(on),
            _ => self.start_pairing(now),
        }
    }

    fn nav_enabled(&self, tab: Tab) -> bool {
        !self.off() || tab.when_off()
    }

    /// The top bar, left to right (and so in Tab order): the logo, the
    /// pages, then at the right the update pill, the status line, Search
    /// and the action.
    pub(super) fn top_bar(&mut self, ui: &mut Ui, bar_r: Rect, now: f64) {
        let p = ui.painter().clone();
        p.rect_filled(bar_r, 0.0, pal().bg);
        p.hline(bar_r.x_range(), bar_r.bottom() - 0.5, Stroke::new(1.0, pal().line));
        let mut ui = ui.new_child(UiBuilder::new().max_rect(bar_r).id_salt("top-bar"));
        ui.set_clip_rect(bar_r);
        let cy = bar_r.center().y;
        let inset = if chrome::mac_frame() { MAC_INSET } else { 0.0 };
        let (label, look, enabled, job) = self.action();
        let (tone, status_text) = self.status_line();
        let pill = self.update_pill();
        let text = BarText { action: &label, status: Some(&status_text), pill };
        let measure = |s: &str, f: &FontId| text_width(&ui, s, f.clone());
        let bar = plan_bar(bar_r.width(), &text, inset, &measure);
        let mut over_control = false;

        // The logo: the flat keycap and the wordmark.
        let mut x = bar_r.left() + inset.max(PAD_X);
        icons::symbol(&p, pos2(x, cy - SYMBOL / 2.0), SYMBOL);
        let mut logo_w = SYMBOL;
        if bar.wordmark {
            let mut job = egui::text::LayoutJob::default();
            job.append("oneMK", 0.0, egui::TextFormat { font_id: wordmark(T_WORDMARK), color: pal().ink, extra_letter_spacing: WORDMARK_TRACK, ..Default::default() });
            let g = p.layout_job(job);
            p.galley(pos2(x + SYMBOL + 9.0, cy - g.size().y / 2.0), g, pal().ink);
            logo_w += 9.0 + wordmark_w(&measure);
        }
        a11y_text(&ui, Id::unique("logo"), Rect::from_min_size(pos2(x, cy - 12.0), vec2(logo_w, 24.0)), Role::Image, "oneMK", None);
        x += logo_w + GAP;

        // The pages.
        let trouble = self.health_trouble();
        for (tab, _) in NAV {
            let w = tab_w(&measure, tab, &bar);
            let r = Rect::from_min_size(pos2(x, cy - CONTROL_H / 2.0), vec2(w, CONTROL_H));
            let look = TabLook { chosen: self.tab == tab, enabled: self.nav_enabled(tab), dot: tab == Tab::Health && trouble };
            let resp = self.nav_tab(&ui, r, tab, nav_label(tab, bar.short_nav), look);
            over_control |= resp.hovered();
            if resp.clicked() {
                self.set_tab(tab);
            }
            x += w + TAB_GAP;
        }

        // The right side, laid out from the right edge, made left to right.
        let shown_label = action_label(&label, bar.action_level).to_owned();
        let action_r = {
            let w = action_w(&measure, &shown_label);
            Rect::from_min_size(pos2(bar_r.right() - PAD_X - w, cy - CONTROL_H / 2.0), vec2(w, CONTROL_H))
        };
        let search_r = {
            let w = search_w(&measure, bar.search_word, bar.search_chip);
            Rect::from_min_size(pos2(action_r.left() - RIGHT_GAP - w, cy - CONTROL_H / 2.0), vec2(w, CONTROL_H))
        };
        let mut left = search_r.left();
        let status_r = bar.status.then(|| {
            let w = status_w(&measure, &status_text);
            left -= RIGHT_GAP + w;
            Rect::from_min_size(pos2(left, cy - 12.0), vec2(w - 4.0, 24.0))
        });
        let pill_r = pill.map(|text| {
            let w = pill_w(&measure, pill_text(&bar, text));
            left -= RIGHT_GAP + w;
            Rect::from_min_size(pos2(left, cy - PILL_H / 2.0), vec2(w, PILL_H))
        });

        if let (Some(r), Some(text)) = (pill_r, pill) {
            let id = Id::unique("update-pill");
            let resp = ui.interact(widgets::hit_rect(r), id, widgets::sense(&ui, Sense::click()));
            let hover = tween(ui.ctx(), id.with("hover"), f32::from(resp.hovered()), 0.1, motion::out_cubic);
            let fill = if resp.is_pointer_button_down_on() { pal().pressed(pal().accent_soft, 0.4) } else { mix(pal().accent_soft, pal().pressed(pal().accent_soft, 0.4), 0.5 * hover) };
            p.rect_filled(r, CornerRadius::same((PILL_H / 2.0) as u8), fill);
            p.text(r.center(), Align2::CENTER_CENTER, pill_text(&bar, text), medium(T_META), pal().accent_text);
            ring(&ui, &resp, r, (PILL_H / 2.0) as u8);
            widgets::a11y(&resp, Role::Button, &format!("{text}: open Settings"), None);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            over_control |= resp.hovered();
            let tip = match (&self.update_available, &self.update_output) {
                (Some(version), Some(out)) => format!("Update oneMK to {version}\n\n{}", out.trim()),
                (Some(version), None) => format!("Update oneMK to {version}"),
                (None, Some(out)) => out.trim().to_owned(),
                (None, None) => text.to_owned(),
            };
            if resp.on_hover_text(tip).clicked() {
                self.set_tab(Tab::Settings);
            }
        }

        if let Some(r) = status_r {
            let id = Id::unique("health-line");
            let resp = ui.interact(widgets::hit_rect(r), id, widgets::sense(&ui, Sense::click()));
            let hover = tween(ui.ctx(), id.with("hover"), f32::from(resp.hovered()), 0.1, motion::out_cubic);
            widgets::status_mark(&p, pos2(r.left() + 3.5, cy), 7.0, widgets::Mark::of(tone));
            p.text(pos2(r.left() + 14.0, cy), Align2::LEFT_CENTER, &status_text, regular(T_META), mix(pal().ink2, pal().ink, hover));
            ring(&ui, &resp, r.expand2(vec2(4.0, 0.0)), 6);
            widgets::a11y(&resp, Role::Link, &format!("{status_text}. Open Health"), None);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            over_control |= resp.hovered();
            if resp.clicked() {
                self.set_tab(Tab::Health);
            }
        }

        // Search: a button that opens the command palette (where the typing
        // happens, in a real text field), not a text field itself.
        {
            let id = Id::unique("search");
            let resp = ui.interact(widgets::hit_rect(search_r), id, widgets::sense(&ui, Sense::click()));
            let hover = tween(ui.ctx(), id.with("hover"), f32::from(resp.hovered()), 0.1, motion::out_cubic);
            let fill = if resp.is_pointer_button_down_on() { widgets::pressed(pal().surface2_hover, 1.0) } else { mix(pal().surface2, pal().surface2_hover, hover) };
            let radius = widgets::button_radius(CONTROL_H);
            p.rect_filled(search_r, CornerRadius::same(radius), fill);
            let ink = mix(pal().ink2, pal().ink, hover);
            icons::search(&p, pos2(search_r.left() + 10.0 + 7.5, cy), ink);
            let mut sx = search_r.left() + 10.0 + 15.0;
            if bar.search_word {
                p.text(pos2(sx + 8.0, cy), Align2::LEFT_CENTER, "Search", regular(T_BODY), ink);
                sx += 8.0 + measure("Search", &regular(T_BODY));
            }
            if bar.search_chip {
                let g = p.layout_no_wrap(search_chip().to_owned(), mono(T_CHIP), pal().ink2);
                let chip = Rect::from_min_size(pos2(sx + 8.0, cy - (g.size().y + 4.0) / 2.0), g.size() + vec2(12.0, 4.0));
                p.rect_filled(chip, CornerRadius::same(5), pal().surface);
                p.galley(chip.center() - g.size() / 2.0, g, pal().ink2);
            }
            ring(&ui, &resp, search_r, radius);
            widgets::a11y(&resp, Role::Button, "Search: open the command palette", None);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            over_control |= resp.hovered();
            if resp.on_hover_text(format!("Search and commands  {}K", cmd())).clicked() {
                self.open_palette(now);
            }
        }

        // The action that matters most right now: one line, never wrapped.
        let shown = !self.sheet_open() && self.status.is_some();
        let a11y_label = match label.as_str() {
            "Take back" => Some("Take back control of this computer".to_owned()),
            "Use keyboard to stop" => Some(format!("Typing on every computer. {}", self.broadcast_stop_hint())),
            l if l != shown_label => Some(l.to_owned()),
            _ => None,
        };
        let resp = button(&mut ui, action_r, Id::unique("action"), &shown_label, look, enabled && shown, a11y_label.as_deref());
        over_control |= resp.hovered();
        let resp = if self.broadcasting() { resp.on_hover_text(self.broadcast_stop_hint()) } else if shown_label != label { resp.on_hover_text(&label) } else { resp };
        if resp.clicked() {
            self.do_action(job, now);
        }

        // macOS: the bar is the title bar, so its empty part moves the window.
        #[cfg(target_os = "macos")]
        if !over_control && ui.input(|i| i.pointer.primary_pressed() && i.pointer.interact_pos().is_some_and(|at| bar_r.contains(at))) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = over_control;
    }

    /// A page's tab: 34 tall, its name in Inter 500 13; chosen, a `surface2`
    /// fill and 600 in `ink`. Health carries a dot when something needs you.
    fn nav_tab(&self, ui: &Ui, r: Rect, tab: Tab, label: &str, look: TabLook) -> egui::Response {
        let full = nav_label(tab, false);
        let id = Id::unique(("nav", full));
        let resp = ui.interact(widgets::hit_rect(r), id, if look.enabled { widgets::sense(ui, Sense::click()) } else { Sense::hover() });
        let hover = tween(ui.ctx(), id.with("hover"), f32::from(look.enabled && resp.hovered()), 0.1, motion::out_cubic);
        let down = look.enabled && resp.is_pointer_button_down_on();
        let p = ui.painter();
        let radius = CornerRadius::same(widgets::button_radius(CONTROL_H));
        if look.chosen {
            p.rect_filled(r, radius, if down { widgets::pressed(pal().surface2, 0.5) } else { pal().surface2 });
        } else if down {
            p.rect_filled(r, radius, widgets::pressed(pal().surface2, 0.3));
        } else if hover > 0.01 {
            // Hover is a hint, clearly weaker than the chosen page.
            p.rect_filled(r, radius, pal().hover.gamma_multiply(hover));
        }
        let (font, col) = if !look.enabled {
            (medium(T_BODY), pal().ink3)
        } else if look.chosen {
            (semibold(T_BODY), pal().ink)
        } else {
            (medium(T_BODY), mix(pal().ink2, pal().ink, hover))
        };
        p.text(r.center(), Align2::CENTER_CENTER, label, font, col);
        if look.dot {
            p.circle_filled(pos2(r.right() - 4.0 - 3.0, r.top() + 7.0 + 3.0), 3.0, pal().warn);
        }
        widgets::focus_ring(ui, &resp, r, widgets::button_radius(CONTROL_H));
        if resp.hovered() && look.enabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let name = if look.dot { format!("{full}, something needs you") } else { full.to_owned() };
        widgets::a11y(&resp, Role::Tab, &name, Some(look.chosen));
        resp.on_hover_text(format!("{full}  {}", shortcut(tab)))
    }

    /// Choose computer: every computer, with whether it is connected, how
    /// fast it answers and where the pointer is. Returns the one chosen.
    pub(super) fn computers_menu(&self, response: &egui::Response, status: &Status, current: Option<&str>) -> Option<String> {
        let pointer = self.pointer_on();
        let mut rows: Vec<(String, String, bool, String)> = vec![("me".into(), status.name.clone(), true, String::new())];
        for peer in &status.peers {
            // The same reading the card shows (its newest), so the two never
            // disagree; the daemon's last link otherwise.
            let value = match status.link_of(&peer.id).filter(|_| peer.online) {
                Some(l) => rtt::ms(self.summary_of(&peer.id).map_or(l.rtt_us, |s| s.now_us)),
                None => String::new(),
            };
            rows.push((peer.id.clone(), peer.name.clone(), peer.online, value));
        }
        let mut selected = None;
        egui::Popup::menu(response).width(300.0).show(|ui| {
            egui::ScrollArea::vertical().max_height((ui.ctx().content_rect().height() - 100.0).max(100.0)).show(ui, |ui| {
                for (id, name, online, value) in &rows {
                    let chosen = current == Some(id.as_str());
                    let here = pointer.as_deref() == Some(id.as_str());
                    // A Mac's host name ends in ".local", which says nothing here.
                    let name = name.strip_suffix(".local").unwrap_or(name);
                    let label = computer_menu_label(name, id, *online, chosen, here, value);
                    let (title, detail) = label.split_once('\n').unwrap();
                    let mut text = egui::text::LayoutJob::default();
                    text.append(&format!("    {title}"), 0.0, egui::TextFormat { font_id: regular(T_BODY), color: pal().ink, ..Default::default() });
                    text.append(&format!("\n    {detail}"), 0.0, egui::TextFormat { font_id: regular(T_META), color: pal().ink2, ..Default::default() });
                    let row = egui::Button::new(text).wrap().min_size(vec2(ui.available_width(), 44.0));
                    let row = if chosen { row.fill(pal().surface2_hover) } else { row };
                    let response = ui.add(row);
                    widgets::a11y(&response, Role::Button, &label, None);
                    widgets::status_mark(ui.painter(), response.rect.left_top() + vec2(10.0, 12.0), 7.0, if *online { widgets::Mark::Ok } else { widgets::Mark::Off });
                    if response.clicked() {
                        selected = Some(id.clone());
                        ui.close();
                    }
                }
            });
        });
        selected
    }

    /// A page's title row: the title, a quieter word after it, and the
    /// room left on the right for the page's own actions.
    pub(super) fn page_head(&self, ui: &Ui, content: Rect, title: &str, sub: &str) -> Rect {
        let r = Rect::from_min_size(content.min + vec2(4.0, 0.0), vec2(content.width() - 4.0, HEAD_H));
        let p = ui.painter();
        // Compact, as a tool's title bar: the page, and what is true now
        // beside it, quieter.
        let big = semibold(T_TITLE);
        let tw = text_width(ui, title, big.clone());
        p.text(pos2(r.left(), r.center().y), Align2::LEFT_CENTER, title, big, pal().ink);
        if !sub.is_empty() {
            let small = regular(T_BODY);
            let room = (r.width() * 0.6 - tw - 12.0).max(0.0);
            p.text(pos2(r.left() + tw + 10.0, r.center().y + 0.5), Align2::LEFT_CENTER, fit(p, sub, &small, room), small, pal().ink3);
        }
        a11y_text(ui, Id::unique(("page", title)), r, Role::Heading, &format!("{title} {sub}"), None);
        r
    }
}

/// The window's parts: the bar across the top, the page under it.
pub(super) struct Frame {
    pub bar: Rect,
    pub content: Rect,
}

/// The page's inset from the window's sides (the handoff's 16).
pub(super) const SIDE: f32 = 16.0;

pub(super) fn frame(full: Rect) -> Frame {
    let bar = Rect::from_min_size(full.min, vec2(full.width(), BAR_H));
    let content = Rect::from_min_max(pos2(full.left() + SIDE, bar.bottom() + 4.0), pos2(full.right() - SIDE, full.bottom() - M));
    Frame { bar, content }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computer_menu_names_include_connection_pointer_and_selection() {
        assert_eq!(computer_menu_label("Gaming PC", "peer", false, true, false, ""), "Gaming PC\nOffline · Selected");
        assert_eq!(computer_menu_label("Desk", "me", true, false, true, ""), "Desk\nThis computer · Pointer here");
        assert_eq!(computer_menu_label("工作站", "peer", true, false, false, "4 ms"), "工作站\nConnected · 4 ms");
    }

    #[test]
    fn every_page_has_its_number_in_the_bar_order() {
        let keys = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7];
        let order: Vec<Tab> = keys.iter().map(|k| tab_for_key(*k).expect("a page")).collect();
        assert_eq!(order, [Tab::Desk, Tab::Arrange, Tab::Wallpaper, Tab::Files, Tab::Keys, Tab::Health, Tab::Settings]);
        assert_eq!(tab_for_key(Key::Comma), Some(Tab::Settings));
        assert_eq!(tab_for_key(Key::Num8), None);
        for key in PAGE_KEYS {
            assert!(tab_for_key(key).is_some());
        }
        let k = cmd();
        assert_eq!(shortcut(Tab::Desk), format!("{k}1"));
        assert_eq!(shortcut(Tab::Arrange), format!("{k}2"));
        assert_eq!(shortcut(Tab::Health), format!("{k}6"));
        assert_eq!(shortcut(Tab::Settings), format!("{k},"));
    }

    #[test]
    fn compact_names_are_short() {
        let names: Vec<&str> = NAV.iter().map(|(t, _)| nav_label(*t, false)).collect();
        assert_eq!(names, ["Desk", "Arrange", "Wallpaper", "Clipboard & files", "Keys & mouse", "Health", "Settings"]);
        let short: Vec<&str> = NAV.iter().map(|(t, _)| nav_label(*t, true)).collect();
        assert_eq!(short, ["Desk", "Arrange", "Wallpaper", "Files", "Keys", "Health", "Settings"]);
    }

    #[test]
    fn the_action_shortens_without_changing_what_it_says() {
        assert_eq!(action_label("Pair a computer", 0), "Pair a computer");
        assert_eq!(action_label("Pair a computer", 2), "Pair");
        assert_eq!(action_label("Pair another computer", 1), "Pair another");
        assert_eq!(action_label("Pair another computer", 2), "Pair");
        assert_eq!(action_label("Use keyboard to stop", 2), "Keyboard stop");
        for label in ["Start oneMK", "Take back", "Stopping…", "Starting…", "Taking back…"] {
            assert_eq!(action_label(label, 2), label);
        }
    }

    /// The handoff's breakpoints, and at every size from the smallest
    /// window up, the bar fits: no word wraps or is cut.
    #[test]
    fn the_bar_fits_at_every_width() {
        let context = egui::Context::default();
        crate::style::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            let m = |s: &str, f: &FontId| text_width(ui, s, f.clone());
            let plain = BarText { action: "Pair a computer", status: Some("On · connected"), pill: None };
            let b = plan_bar(820.0, &plain, 0.0, &m);
            assert!(b.short_nav && b.action_level == 2 && !b.status && !b.search_word, "compact at 820: {b:?}");
            let b = plan_bar(1000.0, &plain, 0.0, &m);
            assert!(!b.short_nav && !b.search_word && !b.status, "1000: {b:?}");
            let b = plan_bar(1100.0, &plain, 0.0, &m);
            assert!(b.search_word && !b.status && b.action_level == 0, "1100: {b:?}");
            let b = plan_bar(1280.0, &plain, 0.0, &m);
            assert!(b.status && b.search_word && b.tab_pad == 12.0 && b.wordmark, "1280: {b:?}");
            for action in ["Pair a computer", "Pair another computer", "Start oneMK", "Take back", "Stopping…", "Use keyboard to stop", "Taking back…"] {
                for pill in [None, Some("Update ready"), Some("Update didn't finish")] {
                    for status in ["On · connected", "On · check Health", "Looking for oneMK…"] {
                        let t = BarText { action, status: Some(status), pill };
                        for inset in [0.0, MAC_INSET] {
                            for width in [820.0, 860.0, 900.0, 999.0, 1000.0, 1050.0, 1099.0, 1100.0, 1200.0, 1279.0, 1280.0, 1440.0, 1920.0] {
                                let b = plan_bar(width, &t, inset, &m);
                                let need = bar_need(&b, &t, inset, &m);
                                assert!(need <= width, "{action} / {pill:?} / {status} at {width} (inset {inset}): needs {need}, {b:?}");
                            }
                        }
                    }
                }
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_page_starts_under_the_bar() {
        let f = frame(Rect::from_min_size(Pos2::ZERO, vec2(820.0, 540.0)));
        assert_eq!(f.bar.height(), BAR_H);
        assert!(f.content.top() >= f.bar.bottom());
        assert_eq!(f.content.left(), SIDE);
        assert_eq!(f.content.right(), 820.0 - SIDE);
    }
}
