//! Whole-window tests: the real `App` is drawn frame after frame in a headless
//! egui `Context`, fed synthetic input, and its output is read back the way a
//! screen reader and the eye read it: the accessibility tree (names, roles,
//! rectangles) and the text that was painted (and the rectangle it was clipped
//! to). No window, no GPU, no daemon: the app's link is `Link::detached`, whose
//! `one-mk` does not exist, and its folder is private, so nothing pressed here
//! can touch a real oneMK, login item or desktop.
//!
//! `egui_kittest` is not a dependency (it is not in the locked set); this is
//! the same idea on plain egui.

use super::*;
use egui::accesskit::{self, NodeId};
use egui::epaint::{ClippedShape, Shape};
use one_mk_ctl::status::TransferStatus;
use std::sync::mpsc::Sender;

/// One thing a screen reader would find: its role, its name, where it is.
#[derive(Clone, Debug)]
pub(super) struct Item {
    pub role: accesskit::Role,
    pub name: String,
    pub rect: Rect,
    pub disabled: bool,
    pub focused: bool,
    /// It has a label of its own (a text field's name is its text otherwise).
    pub named: bool,
}

/// Some text that was painted: what it says, where, and the clip it was cut to.
#[derive(Clone, Debug)]
pub(super) struct Painted {
    pub text: String,
    pub rect: Rect,
    pub clip: Rect,
}

/// A file another program let go over the window: its path only.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

pub(super) struct Win {
    pub ctx: egui::Context,
    pub app: App,
    pub tx: Sender<Event>,
    frame: eframe::Frame,
    pub t: f64,
    pub size: egui::Vec2,
    ppp: f32,
    pub dir: PathBuf,
    events: Vec<egui::Event>,
    dropped: Vec<egui::DroppedFileHandle>,
    pointer: Pos2,
    nodes: HashMap<NodeId, accesskit::Node>,
    root: Option<NodeId>,
    focus: Option<NodeId>,
    pub texts: Vec<Painted>,
    pub frames: u32,
    /// Every shape of the last frame (clip and all).
    pub shapes: Vec<ClippedShape>,
    /// The controls as the last frame without a sheet or palette drew them:
    /// what lies under one that is up now.
    behind: Vec<Item>,
}

static SERIAL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn flatten(shapes: &[ClippedShape], out: &mut Vec<Painted>) {
    fn walk(shape: &Shape, clip: Rect, out: &mut Vec<Painted>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, out)),
            Shape::Text(t) => {
                let rect = t.galley.rect.translate(t.pos.to_vec2());
                out.push(Painted { text: t.galley.text().to_owned(), rect, clip });
            }
            _ => {}
        }
    }
    for s in shapes {
        walk(&s.shape, s.clip_rect, out);
    }
}

impl Win {
    /// A window `w` × `h` points, light or dark, at `ppp` pixels per point.
    pub fn new(w: f32, h: f32, dark: bool, ppp: f32) -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::style::install(&ctx);
        let dir = std::env::temp_dir().join(format!("omk-win-{}-{}", std::process::id(), SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let link = Link::detached(dir.clone(), tx.clone(), ctx.clone());
        let mut app = App::assemble(ctx.clone(), link, rx, [0.0, 0.0]);
        app.prefs.appearance = if dark { crate::style::Appearance::Dark } else { crate::style::Appearance::Light };
        let mut win = Self {
            ctx,
            app,
            tx,
            frame: eframe::Frame::_new_kittest(),
            t: 10.0,
            size: vec2(w, h),
            ppp,
            dir,
            events: vec![],
            dropped: vec![],
            pointer: pos2(-10.0, -10.0),
            nodes: HashMap::new(),
            root: None,
            focus: None,
            texts: vec![],
            frames: 0,
            shapes: vec![],
            behind: vec![],
        };
        // The first frame says "looking for oneMK": no daemon has answered yet.
        win.settle();
        win
    }

    pub fn light(w: f32, h: f32) -> Self {
        Self::new(w, h, false, 1.0)
    }

    /// Tells the window what the daemon would: this status (`None`: not running).
    pub fn status(&mut self, status: Option<Status>) {
        self.tx.send(Event::Status(Box::new(status))).unwrap();
        self.settle();
    }

    pub fn send(&mut self, event: Event) {
        self.tx.send(event).unwrap();
        self.settle();
    }

    /// One frame, 1/60 s on.
    pub fn frame(&mut self) {
        self.t += 1.0 / 60.0;
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.t),
            predicted_dt: 1.0 / 60.0,
            focused: true,
            events: std::mem::take(&mut self.events),
            dropped_files: std::mem::take(&mut self.dropped),
            ..Default::default()
        };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(self.ppp);
        eframe::App::raw_input_hook(&mut self.app, &self.ctx, &mut input);
        let app = &mut self.app;
        let frame = &mut self.frame;
        let mut out = self.ctx.run_ui(input, |ui| eframe::App::ui(app, ui, frame));
        out.textures_delta.clear();
        self.frames += 1;
        if std::env::var_os("OMK_TRACE").is_some() {
            eprintln!("frame {} done at {:?}", self.frames, std::time::SystemTime::now());
        }
        if let Some(update) = out.platform_output.accesskit_update {
            self.nodes = update.nodes.into_iter().collect();
            self.root = update.tree.map(|t| t.root);
            self.focus = Some(update.focus);
        }
        self.texts.clear();
        flatten(&out.shapes, &mut self.texts);
        self.shapes = out.shapes;
        if !self.app.sheet_open() && !self.app.palette.open {
            self.behind = self.controls();
        }
    }

    pub fn frames(&mut self, n: u32) {
        for _ in 0..n {
            self.frame();
        }
    }

    /// Frames until the page, the desk and the sheet have come to rest
    /// (three quarters of a second; the window's animations are shorter).
    pub fn settle(&mut self) {
        self.frames(45);
    }

    /// Opening Health starts a real check on a thread (no daemon here, so it ends in an error); a slow
    /// computer can deliver its result after a test has sent its own, and until it does the window's
    /// "Check again" and "Open guide" buttons are off. Waits for it (at most 30 s) so what a test sends next is last.
    pub fn health_check_ends(&mut self) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while self.app.busy.contains(&crate::daemon::Job::Doctor) && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(20));
            self.frames(1);
        }
    }

    /// Files let go over the window, at the pointer.
    pub fn drop_files(&mut self, files: &[PathBuf]) {
        self.dropped = files.iter().map(|f| std::sync::Arc::new(Dropped(f.clone())) as egui::DroppedFileHandle).collect();
        self.frames(2);
    }

    pub fn push(&mut self, event: egui::Event) {
        self.events.push(event);
    }

    pub fn move_to(&mut self, p: Pos2) {
        self.pointer = p;
        self.push(egui::Event::PointerMoved(p));
        self.frames(2);
    }

    pub fn click_at(&mut self, p: Pos2) {
        self.move_to(p);
        for pressed in [true, false] {
            self.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            self.frames(2);
        }
        self.frames(20);
    }

    /// Press and let go, and stop at that frame: what the click did at once is
    /// still what is on show (a failing command comes back a few frames on).
    pub fn click_now(&mut self, name: &str) {
        let item = self.find(name).unwrap_or_else(|| panic!("no {name:?} on screen; the tree has: {:#?}", self.names()));
        let p = item.rect.center();
        self.move_to(p);
        for pressed in [true, false] {
            self.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            self.frame();
        }
    }

    pub fn right_click_at(&mut self, p: Pos2) {
        self.move_to(p);
        for pressed in [true, false] {
            self.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
            self.frames(2);
        }
        self.frames(20);
    }

    pub fn drag(&mut self, from: Pos2, to: Pos2) {
        self.move_to(from);
        self.push(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        self.frames(2);
        for k in 1..=8 {
            self.move_to(from + (to - from) * (k as f32 / 8.0));
        }
        self.push(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        self.frames(20);
    }

    pub fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.push(egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers });
        self.frames(2);
        self.push(egui::Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers });
        self.frames(10);
    }

    pub fn press(&mut self, key: Key) {
        self.key(key, Modifiers::NONE);
    }

    pub fn shortcut(&mut self, key: Key) {
        self.key(key, Modifiers::COMMAND);
    }

    pub fn type_text(&mut self, text: &str) {
        self.push(egui::Event::Text(text.to_owned()));
        self.frames(4);
    }

    // ---------- reading it back ----------

    /// Everything the tree names, in tree order, that sits somewhere.
    pub fn items(&self) -> Vec<Item> {
        let mut out = vec![];
        let Some(root) = self.root else { return out };
        let mut stack = vec![root];
        let mut seen = HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(node) = self.nodes.get(&id) else { continue };
            for child in node.children().iter().rev() {
                stack.push(*child);
            }
            if node.is_hidden() {
                continue;
            }
            let Some(b) = node.bounds() else { continue };
            let name = node.label().or_else(|| node.value()).unwrap_or("").to_owned();
            out.push(Item {
                role: node.role(),
                name,
                rect: Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32)),
                disabled: node.is_disabled(),
                focused: self.focus == Some(id),
                named: node.label().is_some_and(|l| !l.trim().is_empty()),
            });
        }
        out
    }

    /// The things that can be pressed or switched: by role.
    pub fn controls(&self) -> Vec<Item> {
        use accesskit::Role::*;
        self.items().into_iter().filter(|i| matches!(i.role, Button | CheckBox | RadioButton | Switch | Tab | MenuItem | TextInput | ComboBox | Slider | Link | MenuItemCheckBox | MenuItemRadio)).collect()
    }

    pub fn find(&self, name: &str) -> Option<Item> {
        self.items().into_iter().find(|i| i.name == name)
    }

    pub fn find_containing(&self, part: &str) -> Option<Item> {
        self.items().into_iter().find(|i| i.name.contains(part))
    }

    pub fn has_text(&self, part: &str) -> bool {
        self.texts.iter().any(|t| t.text.contains(part)) || self.items().iter().any(|i| i.name.contains(part))
    }

    pub fn click(&mut self, name: &str) {
        let item = self.find(name).unwrap_or_else(|| panic!("no {name:?} on screen; the tree has: {:#?}", self.names()));
        assert!(!item.disabled, "{name:?} is disabled");
        self.click_at(item.rect.center());
    }

    pub fn names(&self) -> Vec<String> {
        self.items().iter().filter(|i| !i.name.is_empty()).map(|i| format!("{:?} {:?} {:?}", i.role, i.name, i.rect)).collect()
    }

    pub fn window(&self) -> Rect {
        Rect::from_min_size(Pos2::ZERO, self.size)
    }

    /// Resize the window (as a drag of its edge would), then let it settle.
    pub fn resize(&mut self, w: f32, h: f32) {
        self.size = vec2(w, h);
        self.settle();
    }

    pub fn tab(&self) -> Tab {
        self.app.tab
    }

    pub fn go(&mut self, tab: Tab) {
        let key = shell::NAV.iter().find(|n| n.0 == tab).map(|n| n.1).unwrap_or("1");
        let key = match key {
            "1" => Key::Num1,
            "2" => Key::Num2,
            "3" => Key::Num3,
            "4" => Key::Num4,
            "5" => Key::Num5,
            "6" => Key::Num6,
            _ => Key::Num7,
        };
        self.shortcut(key);
        self.settle();
        assert_eq!(self.app.tab, tab, "the shortcut for {tab:?}");
    }
}

impl Drop for Win {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---------- fixtures: what the daemon says ----------

pub(super) fn screen(x: i32, y: i32, w: i32, h: i32) -> one_mk_proto::Rect {
    one_mk_proto::Rect::new(x, y, w, h)
}

pub(super) fn peer(id: &str, name: &str, side: Side, os: u8, online: bool) -> PeerStatus {
    PeerStatus { id: id.into(), name: name.into(), side, os, online, swap: "auto".into(), screens: if online { vec![screen(0, 0, 1920, 1080)] } else { vec![] } }
}

/// Running, nobody paired.
pub(super) fn alone() -> Status {
    Status { name: "Studio".into(), id: "me".into(), port: 24800, screens: vec![screen(0, 0, 2560, 1440)], gaming: "auto".into(), typing: "auto".into(), hotkeys: "on".into(), ..Default::default() }
}

/// Running, with these computers paired.
pub(super) fn with(peers: Vec<PeerStatus>) -> Status {
    Status { peers, ..alone() }
}

/// This computer and one online computer on its right.
pub(super) fn two() -> Status {
    with(vec![peer("p1", "Office PC", Side::Right, os::WINDOWS, true)])
}

/// Many computers, of every kind, with monitors of different shapes.
pub(super) fn crowded() -> Status {
    let mut peers = vec![
        peer("p1", "Office PC", Side::Right, os::WINDOWS, true),
        peer("p2", "MacBook Pro", Side::Left, os::MACOS, true),
        peer("p3", "Living room", Side::Top, os::LINUX, true),
        peer("p4", "Old laptop", Side::Bottom, os::LINUX, false),
        peer("p5", "Build server", Side::Right, os::LINUX, true),
    ];
    peers[0].screens = vec![screen(0, 0, 1920, 1080), screen(1920, 0, 1920, 1080)];
    peers[1].screens = vec![screen(0, 0, 1512, 982)];
    let mut status = with(peers);
    status.screens = vec![screen(0, 0, 2560, 1440), screen(-1080, -240, 1080, 1920)];
    status
}

/// Names that are hard on a layout: very long, in other scripts, right to left.
pub(super) fn hard_names() -> Status {
    let mut status = with(vec![
        peer("p1", "The very long name of the computer in the corner of the big office upstairs", Side::Right, os::WINDOWS, true),
        peer("p2", "日本語のコンピュータ名前", Side::Left, os::MACOS, true),
        peer("p3", "مكتب العمل الرئيسي", Side::Top, os::LINUX, true),
        peer("p4", "שלום עולם מחשב", Side::Bottom, os::LINUX, false),
    ]);
    status.name = "Мой компьютер 🖥️ with a long name that goes on".into();
    status
}

/// What was painted with a rectangle that runs past the clip it was drawn in
/// (text cut off at the side), or off the window. `slack`: points allowed.
pub(super) fn cut_off(win: &Win, slack: f32) -> Vec<String> {
    let window = win.window().expand(slack);
    win.texts
        .iter()
        .filter(|t| !t.text.trim().is_empty() && t.rect.width() > 0.0)
        .filter(|t| {
            let clip = t.clip.intersect(window);
            t.rect.left() < clip.left() - slack || t.rect.right() > clip.right() + slack
        })
        // Text that is wholly outside the clip is not drawn at all (a scrolled-off row): fine.
        .filter(|t| t.clip.intersects(t.rect) && win.window().intersects(t.rect))
        .map(|t| format!("{:?} {:?} clip {:?}", t.text, t.rect, t.clip))
        .collect()
}

/// What is wrong with the screen as drawn: a list of plain sentences, empty
/// when it is right. Checks what any page must keep: every control has a name
/// and sits in the window; no two controls lie on each other; no text is cut
/// off at the side of the box it was drawn in.
pub(super) fn problems(win: &Win) -> Vec<String> {
    let mut out = vec![];
    let window = win.window();
    let controls = win.controls();
    for c in &controls {
        if c.name.trim().is_empty() || (c.role == accesskit::Role::TextInput && !c.named) {
            out.push(format!("a {:?} at {:?} has no name for a screen reader", c.role, c.rect));
        }
        if c.rect.width() < 1.0 || c.rect.height() < 1.0 {
            continue;
        }
        // Pages scroll down, never sideways: a control past the left or right
        // edge is cut off. (One scrolled below the fold is fine.)
        if c.rect.left() < window.left() - 1.0 || c.rect.right() > window.right() + 1.0 {
            out.push(format!("{:?} {:?} runs past the side of the window {:?}", c.role, c.name, c.rect));
        }
    }
    // The drawn keyboard's keys are one hit area each, 24 points at least:
    // they overlap on purpose where the drawing is small.
    let controls: Vec<Item> = controls.into_iter().filter(|c| !c.name.contains("Arrows move between keys")).collect();
    // A sheet or the palette lies over the page on purpose (and holds the
    // keyboard): the controls under it are not on show, so only what is new
    // since it came up is compared.
    let modal = win.app.palette.open || win.app.sheet_open();
    let controls: Vec<Item> = if modal {
        // (By name: the desk may move over to make room for the sheet.)
        let under = |c: &Item| win.behind.iter().any(|b| b.name == c.name);
        controls.into_iter().filter(|c| !under(c)).collect()
    } else {
        controls
    };
    for (i, a) in controls.iter().enumerate() {
        for b in &controls[i + 1..] {
            let both = a.rect.intersect(b.rect);
            if both.width() <= 1.0 || both.height() <= 1.0 {
                continue;
            }
            let smaller = a.rect.area().min(b.rect.area());
            if both.area() > 0.25 * smaller && !a.rect.contains_rect(b.rect) && !b.rect.contains_rect(a.rect) {
                out.push(format!("{:?} {:?} {:?} lies on {:?} {:?} {:?}", a.role, a.name, a.rect, b.role, b.name, b.rect));
            }
        }
    }
    out.extend(cut_off(win, 1.5).into_iter().map(|t| format!("text cut off: {t}")));
    out
}

/// Buttons whose accessible name does not contain the words painted on them
/// (WCAG 2.5.3, label in name: a person who says "click Try again" must reach
/// the button, whatever a screen reader reads out).
pub(super) fn label_not_in_name(win: &Win) -> Vec<String> {
    use accesskit::Role::*;
    let mut out = vec![];
    for c in win.controls().into_iter().filter(|c| c.role == Button) {
        // The keys of the drawing, and the blocks of the plan (which can lie over each other).
        if c.name.contains("Arrows move between keys") || c.name.contains("drag it to place it") {
            continue;
        }
        let name = c.name.to_lowercase();
        for t in &win.texts {
            let word = t.text.trim().to_lowercase();
            let letters = word.chars().filter(|ch| ch.is_alphabetic()).count();
            // Painted wholly inside the control, readable, not shortened with an ellipsis.
            // A shortcut painted on a button ("Ctrl+Alt+P", "Control+Option+P" on a Mac) is its value, not its label.
            let chord = word.contains("ctrl") || (word.contains('+') && ["control", "alt", "option", "shift", "super", "win"].iter().any(|m| word.contains(m)));
            if letters >= 2 && !word.contains('…') && !chord && word != "selected" && !word.contains('⌘') && c.rect.contains_rect(t.rect.shrink(0.5)) && t.clip.intersects(c.rect) && !name.contains(&word) {
                out.push(format!("{:?} {:?} has {:?} painted on it", c.role, c.name, t.text));
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod flows;
#[cfg(test)]
mod pages;
// The click-through flows use a shell script as `one-mk` and a unix socket as the daemon.
#[cfg(unix)]
mod qa;
#[cfg(unix)]
mod qa_files;
#[cfg(unix)]
mod qa_settings;
#[cfg(unix)]
mod qa_updates_settings;
#[cfg(unix)]
mod qa_wallpaper;
