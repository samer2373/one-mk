//! What each setting means. A short caption sits under its label; an "i"
//! beside it opens the full sentence and a small looping picture of the
//! setting at work. The pictures are drawn, not recorded: five scenes (a
//! pointer crossing, something copied, two computers acting as one, a
//! shortcut, a notice) with a variant each, so a change to the window
//! never leaves a stale video behind. Reduced motion shows the last frame.

use crate::motion;
use crate::style::*;
use crate::widgets::mix;
use egui::{Align2, Color32, CornerRadius, Painter, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scene {
    /// Nothing worth drawing: the sentence says it all.
    None,
    Cross(Cross),
    Copy(Item),
    Mirror(Both),
    Keys,
    Notice,
    Scroll(Roll),
}

/// A scroll wheel at work: the same page shown without and with the setting.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Roll {
    Flip,
    FlipSide,
    Sideways,
    Fast,
    Steps,
}

/// How the pointer meets the edge between two screens.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cross {
    Locked,
    Fullscreen,
    Corner,
    Dwell,
    HoldKey,
    /// Out through one end of the desk, in at the other.
    Wrap,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    Text,
    Rich,
    Image,
    /// Marked sensitive by its app: it stays where it was copied.
    Secret,
    /// Pulled by the other computer, not pushed.
    Fetch,
}

/// What happens on both screens at once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Both {
    Awake,
    Sleep,
    Lock,
}

pub struct Help {
    /// Under the label: one line, at most `CAPTION_MAX` characters.
    pub caption: &'static str,
    /// In the popover: what it does, and what turning it off changes.
    pub detail: &'static str,
    pub scene: Scene,
}

#[cfg(test)]
const CAPTION_MAX: usize = 46;

use Scene::*;

/// Keyed by the same name the control's id is built from.
pub static HELP: &[(&str, Help)] = &[
    ("autostart", Help { caption: "Opens oneMK when you sign in", detail: "oneMK starts by itself when you sign in to this computer, so the keyboard and mouse are already shared. Off: you open it yourself.", scene: None }),
    ("lock-pointer", Help { caption: "Edges stop leading to other computers", detail: "While this is on, the pointer stays on this computer, even at an edge. Handy when you are playing or working here and don't want to slip across by accident.", scene: Cross(self::Cross::Locked) }),
    ("clip-text", Help { caption: "Copied text appears on other computers", detail: "Copy text here and paste it on another paired computer. Off: text stays on the computer you copied it on.", scene: Copy(Item::Text) }),
    ("clip-html", Help { caption: "Keeps bold, links and colors", detail: "Formatted text keeps its formatting when you paste it on another computer. Off: it arrives as plain text.", scene: Copy(Item::Rich) }),
    ("clip-image", Help { caption: "Copied pictures travel too", detail: "Copy a picture or a screenshot here and paste it on another computer. Off: only text is shared.", scene: Copy(Item::Image) }),
    ("allow-fetch", Help { caption: "Other computers can ask for your clipboard", detail: "Any paired computer can copy this clipboard with Get clipboard, including passwords you copied. Off: only what you send yourself leaves this computer.", scene: Copy(Item::Fetch) }),
    ("notify", Help { caption: "A notice when a file lands on this computer", detail: "A notification appears when someone sends you a file. Off: files still arrive, silently.", scene: Notice }),
    ("edge-corner", Help { caption: "Keeps corners easy to hit", detail: "The pointer won't cross within this many pixels of a corner, so you can reach a corner or a close button without sliding onto the next screen. 0 turns it off.", scene: Cross(self::Cross::Corner) }),
    ("edge-dwell", Help { caption: "Rest this long to cross", detail: "The pointer has to stay against the edge this many milliseconds before it crosses. A short wait stops accidental crossings. 0 crosses at once.", scene: Cross(self::Cross::Dwell) }),
    ("edge-mod", Help { caption: "Cross only while held", detail: "Pick a key. The pointer crosses to the next computer only while you hold it. None: it crosses whenever it reaches the edge.", scene: Cross(self::Cross::HoldKey) }),
    ("edge-fullscreen", Help { caption: "No crossing during fullscreen games or video", detail: "Don't cross while a fullscreen window (a game, a video) has the focus here. Off: the pointer crosses even then.", scene: Cross(self::Cross::Fullscreen) }),
    ("block-saver", Help { caption: "Computers you control never sleep", detail: "No screensaver or sleep on a computer while this one controls it. Off: each computer follows its own power settings.", scene: Mirror(Both::Awake) }),
    ("sleep-sync", Help { caption: "Displays sleep and wake as one", detail: "When this display goes to sleep or wakes, the other computers' displays do too.", scene: Mirror(Both::Sleep) }),
    ("lock-sync", Help { caption: "Locking this one locks them all", detail: "When you lock this computer, every connected computer locks as well.", scene: Mirror(Both::Lock) }),
    ("gaming", Help { caption: "Raw mouse input", detail: "On another computer, send raw mouse movement (for games). Auto: only while it shows a fullscreen window. Off: normal pointer movement everywhere.", scene: None }),
    ("scroll-rev-v", Help { caption: "Scroll up moves the page the other way", detail: "Flips up and down for scrolling on the other computers. Handy when a mouse wheel feels backwards compared with your touchpad.", scene: Scroll(Roll::Flip) }),
    ("scroll-rev-h", Help { caption: "Scroll left moves the page the other way", detail: "Flips left and right for scrolling on the other computers.", scene: Scroll(Roll::FlipSide) }),
    ("scroll-shift", Help { caption: "Shift plus wheel scrolls across", detail: "Holding Shift while you scroll moves the page sideways instead of up and down.", scene: Scroll(Roll::Sideways) }),
    ("scroll-alt", Help { caption: "Hold a key to scroll in big jumps", detail: "Holding the key while you scroll makes each notch move five times as far.", scene: Scroll(Roll::Fast) }),
    ("scroll-mode", Help { caption: "Smooth glides, notched moves a line at a time", detail: "As it comes: steps are passed on as the device sends them. Smooth: notches become fine pixel movement. Notched: whole lines at a time.", scene: Scroll(Roll::Steps) }),
    ("scroll-natural", Help { caption: "Which way the page follows your fingers", detail: "As here: scrolling feels on the other computers as it does here. Match each: natural on Macs, traditional on Windows and Linux. Always: content follows your fingers everywhere. Never: content moves against them everywhere.", scene: None }),
    ("scroll-speed", Help { caption: "How far each scroll goes", detail: "A multiplier on scrolling. 1× leaves it as it is; below 1× is slower, above 1× is faster.", scene: None }),
    ("scroll-device", Help { caption: "Wheels and touchpads can differ", detail: "Choose whether the scrolling settings below apply to mouse wheels, touchpads, or both.", scene: None }),
    ("pointer-speed", Help { caption: "How far the pointer moves", detail: "A multiplier on pointer movement. 1× leaves it as it is; below 1× is slower, above 1× is faster.", scene: None }),
    ("pointer-accel", Help { caption: "Whether quick movements go further", detail: "Like here: use this computer's acceleration. Off: the pointer moves in proportion to your hand, however fast. Curve: the faster you move, the further it goes. Off and Curve need raw mouse movement, which isn't available on every system.", scene: None }),
    ("clip-sensitive", Help { caption: "Passwords stay on this computer", detail: "Passwords and other copies their app marked as sensitive aren't pushed to other computers or handed out when another computer asks for your clipboard. Sending one yourself is refused unless you add --allow-sensitive on the command line.", scene: Copy(Item::Secret) }),
    ("file-drop-zones", Help { caption: "Drop a file on a screen edge to send it", detail: "Experimental. Hold a file over the thin strip at the screen edge that faces another computer and let go: it is copied into that computer's received-files folder. Nothing asks before it sends. Off by default; tried so far only with scripted drags.", scene: Copy(Item::Text) }),
    ("history-enabled", Help { caption: "Remember recent copies", detail: "Keeps a list of what you copied so you can bring an old copy back. Off: nothing you copy is kept. It may keep passwords their app didn't mark as secret.", scene: None }),
    ("history-persist", Help { caption: "Keep the list when oneMK restarts", detail: "Saves the history to disk, unencrypted, so it survives a restart. Off: the history lives in memory only and goes when oneMK stops.", scene: None }),
    ("wallpaper-battery", Help { caption: "Saves power on laptops", detail: "A video wallpaper pauses while a laptop is running on its battery.", scene: None }),
    ("wallpaper-computer", Help { caption: "Whether this computer shows it", detail: "Turn the wallpaper on or off for this computer.", scene: None }),
    ("hook-shell", Help { caption: "Pipes and && work in the command", detail: "Runs the command through the computer's shell, so pipes, `&&` and variables work. Off: the program is started directly.", scene: None }),
    ("also-on-this-computer", Help { caption: "Your rules apply here too", detail: "Experimental, not yet tested on real hardware. Apply your key and button rules here as well as on the other computers. Pointer speed and scrolling stay unchanged. Ctrl + Alt + Shift + Esc turns local rules off and brings you back here.", scene: None }),
    ("autoscroll-toggle", Help { caption: "Click a button, then move to scroll", detail: "Click the chosen mouse button, then move away to scroll the other computer. Click again or press Esc to stop.", scene: None }),
    ("autoscroll-speed", Help { caption: "How fast autoscroll goes", detail: "A multiplier on autoscroll. 1× leaves it as it is; below 1× is slower, above 1× is faster.", scene: None }),
    ("update-check", Help { caption: "Checks for a newer oneMK by itself", detail: "oneMK checks whether a newer version is out and tells you. Off: it never checks by itself.", scene: None }),
    ("update-auto", Help { caption: "New versions install by themselves", detail: "When a newer version is found, oneMK installs it without asking. Off: you choose when. Not available where installing needs a password or this copy can't update itself.", scene: None }),
    ("edge-wrap", Help { caption: "Pointer re-enters on the far side", detail: "Off: the pointer stops at the edge of the desk. Left / right: pushing out through a left or right edge with no computer there brings it back in at the opposite edge. All edges: the top and bottom wrap too.", scene: Cross(self::Cross::Wrap) }),
    ("edge-learn", Help { caption: "Works out where computers sit", detail: "Push the pointer twice within 2 seconds against an edge that leads nowhere, while a computer whose place is only a guess is online, and oneMK takes it as \"it's over there\". Ask me: suggests the place and moves nothing. Place them: puts it there and tells you how to undo that. Off: never learns.", scene: None }),
    ("pointer-overlay", Help { caption: "A pointer for computers with no mouse", detail: "A computer with no mouse attached can show no pointer at all while you control it. Auto: oneMK draws a small arrow only when the system shows none. Always: whenever this computer is controlled. Never: not at all.", scene: None }),
    ("broadcast-idle", Help { caption: "Typing on every computer stops itself", detail: "Typing on every computer at once stops after this many minutes without input. 0 means it never stops by itself.", scene: None }),
    ("wallpaper-fit", Help { caption: "How the picture meets the screens", detail: "Fill: the picture covers the screens and crops what doesn't fit. Fit: all of it shows, with bars where the shapes differ. Stretch: it is pulled to the exact shape of the screens, which can distort it.", scene: None }),
    ("hotkeys", Help { caption: "Shortcuts that jump to another computer", detail: "Press a shortcut to send the keyboard and mouse straight to a computer, without moving the pointer to an edge. Off: only the edges switch.", scene: Keys }),
];

pub fn get(key: &str) -> Option<&'static Help> {
    HELP.iter().find(|(k, _)| *k == key).map(|(_, h)| h)
}

/// Where we are in the loop, 0..1; the last frame when motion is reduced.
pub fn phase(now: f64, secs: f64) -> f32 {
    if motion::reduced() { 0.9 } else { ((now % secs) / secs) as f32 }
}

pub fn playback_start(clock: f64, frame: u64, start: f64, seen: u64) -> f64 {
    if frame > seen + 1 { clock } else { start }
}

fn seg(t: f32, a: f32, b: f32) -> f32 {
    ((t - a) / (b - a)).clamp(0.0, 1.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn ease(t: f32) -> f32 {
    motion::in_out_cubic(t)
}

/// Draws `scene` at time `t` (0..1) inside `r`.
pub fn draw(p: &Painter, r: Rect, scene: Scene, t: f32) {
    if scene == Scene::None {
        return;
    }
    let (a, b) = screens(r);
    let pointer = |at: Pos2, col: Color32| {
        p.add(egui::Shape::convex_polygon(
            vec![at, at + vec2(0.0, 14.0), at + vec2(4.0, 10.5), at + vec2(7.5, 15.5), at + vec2(9.5, 14.5), at + vec2(6.5, 9.5), at + vec2(11.0, 9.5)],
            col,
            Stroke::new(1.0, GROUND),
        ));
    };
    match scene {
        Scene::None => {}
        Scene::Cross(c) => cross(p, (a, b), c, t, &pointer),
        Scene::Copy(i) => copy(p, (a, b), i, t),
        Scene::Mirror(m) => {
            screen(p, a, 1.0);
            screen(p, b, 1.0);
            mirror(p, a, m, t);
            mirror(p, b, m, t);
        }
        Scene::Keys => {
            screen(p, a, 1.0);
            screen(p, b, 1.0);
            let on = seg(t, 0.15, 0.3) * (1.0 - seg(t, 0.55, 0.7));
            keycap(p, r.center_bottom() + vec2(0.0, -12.0), "Ctrl + Alt + 2", on);
            let jump = ease(seg(t, 0.4, 0.6));
            let from = a.center() + vec2(-6.0, -6.0);
            let to = b.center() + vec2(-6.0, -6.0);
            pointer(if jump < 0.5 { from } else { to }, if jump < 0.5 { INK } else { SIGNAL });
        }
        Scene::Scroll(k) => roll(p, r, k, t),
        Scene::Notice => {
            screen(p, a, 1.0);
            screen(p, b, 1.0);
            let x = ease(seg(t, 0.15, 0.5));
            file(p, a.center().lerp(b.center(), x), 1.0 - seg(t, 0.5, 0.55));
            let drop = ease(seg(t, 0.55, 0.7)) * (1.0 - seg(t, 0.9, 1.0));
            let banner = Rect::from_min_size(pos2(b.left() + 6.0, b.top() + 6.0 - 20.0 * (1.0 - drop)), vec2(b.width() - 12.0, 14.0));
            p.rect_filled(banner, CornerRadius::same(4), mix(Color32::TRANSPARENT, SIGNAL, drop));
            p.rect_filled(banner.shrink2(vec2(6.0, 5.0)), CornerRadius::same(1), mix(Color32::TRANSPARENT, SIGNAL_INK, drop));
        }
    }
}

/// Two screens with a gap between, side by side in `r`.
fn screens(r: Rect) -> (Rect, Rect) {
    let size = vec2(((r.width() - 56.0) / 2.0).min(120.0), 64.0);
    let y = r.center().y - size.y / 2.0 - 6.0;
    let a = Rect::from_min_size(pos2(r.center().x - 8.0 - size.x, y), size);
    let b = Rect::from_min_size(pos2(r.center().x + 8.0, y), size);
    (a, b)
}

fn screen(p: &Painter, r: Rect, lit: f32) {
    p.rect_filled(r, CornerRadius::same(6), mix(GROUND, RAISED, lit));
    p.rect_stroke(r, CornerRadius::same(6), Stroke::new(1.5, mix(LINE, TRACK_EDGE, lit)), StrokeKind::Inside);
}

fn cross(p: &Painter, (a, b): (Rect, Rect), c: Cross, t: f32, pointer: &dyn Fn(Pos2, Color32)) {
    screen(p, a, 1.0);
    screen(p, b, 1.0);
    let mid = a.center().y;
    let (start, edge, land) = (a.left() + 18.0, a.right() - 3.0, b.left() + 34.0);
    let (mut at, mut col, mut blocked) = (pos2(start, mid), INK, false);
    match c {
        Cross::Locked => {
            at.x = lerp(start, edge, ease(seg(t, 0.1, 0.45)));
            at.x = lerp(at.x, start + 20.0, ease(seg(t, 0.6, 0.85)));
            blocked = t > 0.45 && t < 0.85;
        }
        Cross::Fullscreen => {
            // A window filling the screen: the pointer meets the edge and stays.
            p.rect_filled(a.shrink(4.0), CornerRadius::same(3), PILL);
            p.rect_filled(Rect::from_min_size(a.min + vec2(4.0, 4.0), vec2(a.width() - 8.0, 6.0)), CornerRadius::same(2), BUTTON_LIT);
            at.x = lerp(start, edge, ease(seg(t, 0.1, 0.5)));
            blocked = t > 0.5;
        }
        Cross::Corner => {
            let zone = Rect::from_min_size(pos2(a.right() - 22.0, a.top()), vec2(22.0, 22.0));
            p.rect_filled(zone, CornerRadius { ne: 6, ..Default::default() }, mix(Color32::TRANSPARENT, SIGNAL, 0.22));
            if t < 0.5 {
                at = pos2(lerp(start, edge, ease(seg(t, 0.05, 0.4))), a.top() + 10.0);
                blocked = t > 0.4;
            } else {
                at = pos2(edge - 6.0, lerp(a.top() + 10.0, mid + 6.0, ease(seg(t, 0.5, 0.65))));
                at.x = lerp(at.x, land, ease(seg(t, 0.7, 0.95)));
            }
        }
        Cross::Dwell => {
            at.x = lerp(start, edge, ease(seg(t, 0.05, 0.3)));
            at.x = lerp(at.x, land, ease(seg(t, 0.75, 0.95)));
            let fill = seg(t, 0.3, 0.75);
            if fill > 0.0 && fill < 1.0 {
                ring(p, pos2(edge - 6.0, mid + 6.0), 11.0, fill);
            }
        }
        Cross::Wrap => {
            // Out through the right end, in at the left end.
            let (lo, hi) = (a.left() + 3.0, b.right() - 3.0);
            col = INK;
            if t < 0.45 {
                at.x = lerp(a.left() + 30.0, hi, ease(seg(t, 0.05, 0.45)));
            } else {
                at.x = lerp(lo, a.left() + 30.0, ease(seg(t, 0.6, 0.9)));
                col = SIGNAL;
                if t < 0.6 {
                    at.x = -100.0;
                }
            }
        }
        Cross::HoldKey => {
            let held = seg(t, 0.3, 0.38) * (1.0 - seg(t, 0.85, 0.92));
            keycap(p, pos2(a.center().x, a.bottom() + 20.0), "Ctrl", held);
            at.x = lerp(start, edge, ease(seg(t, 0.05, 0.3)));
            at.x = lerp(at.x, land, ease(seg(t, 0.5, 0.85)));
            blocked = (0.3..0.5).contains(&t) && held < 0.01;
        }
    }
    if at.x > a.right() && c != Cross::Wrap {
        col = SIGNAL;
    }
    // Between the two ends of a wrap the pointer is nowhere.
    if at.x >= 0.0 {
        pointer(at, col);
    }
    if blocked {
        cross_mark(p, pos2(edge + 6.0, at.y + 6.0));
    }
}

fn roll(p: &Painter, r: Rect, k: Roll, t: f32) {
    let s = Rect::from_center_size(r.center() + vec2(-14.0, -6.0), vec2(150.0, 70.0));
    screen(p, s, 1.0);
    let second = t >= 0.5;
    let u = ease(seg(t % 0.5, 0.05, 0.45));
    let (dx, dy, tag, key) = match k {
        Roll::Flip | Roll::FlipSide => {
            let d = if second { 42.0 } else { -42.0 } * u;
            let (dx, dy) = if k == Roll::Flip { (0.0, d) } else { (d, 0.0) };
            (dx, dy, if second { "On" } else { "Off" }, "")
        }
        Roll::Sideways => {
            let (dx, dy) = if second { (-42.0 * u, 0.0) } else { (0.0, -42.0 * u) };
            (dx, dy, "", if second { "Shift" } else { "" })
        }
        Roll::Fast => (0.0, if second { -50.0 } else { -10.0 } * u, "", if second { "Alt" } else { "" }),
        Roll::Steps => (0.0, if second { -14.0 * (u * 3.0).floor().min(3.0) } else { -42.0 * u }, if second { "Notched" } else { "Smooth" }, ""),
    };
    // The page: a lattice of text-like bars, shifted by the scroll.
    let clip = p.with_clip_rect(s.shrink(3.0));
    let (ox, oy) = (dx.rem_euclid(28.0), dy.rem_euclid(14.0));
    for row in -1..7 {
        for col in -1..8 {
            let at = pos2(s.left() + ox + col as f32 * 28.0, s.top() + oy + row as f32 * 14.0 + 4.0);
            clip.rect_filled(Rect::from_min_size(at, vec2(18.0, 4.0)), CornerRadius::same(2), mix(Color32::TRANSPARENT, INK2, 0.5));
        }
    }
    // The wheel, always turned the same way.
    let c = pos2(s.right() + 22.0, s.center().y - 12.0);
    let (a, b) = if k == Roll::FlipSide { (vec2(-7.0, 0.0), vec2(7.0, 0.0)) } else { (vec2(0.0, -8.0), vec2(0.0, 8.0)) };
    let st = Stroke::new(2.0, INK);
    p.line_segment([c + a, c + b], st);
    let (dir, tip) = (b.normalized(), c + b);
    let perp = vec2(-dir.y, dir.x);
    p.line_segment([tip, tip - dir * 4.0 + perp * 4.0], st);
    p.line_segment([tip, tip - dir * 4.0 - perp * 4.0], st);
    if !tag.is_empty() {
        p.text(pos2(s.center().x, s.bottom() + 14.0), Align2::CENTER_CENTER, tag, regular(T_META), if second { SIGNAL } else { INK2 });
    }
    if !key.is_empty() {
        keycap(p, pos2(c.x, s.center().y + 22.0), key, seg(t, 0.55, 0.6) * (1.0 - seg(t, 0.95, 1.0)));
    }
}

fn copy(p: &Painter, (a, b): (Rect, Rect), item: Item, t: f32) {
    screen(p, a, 1.0);
    screen(p, b, 1.0);
    let (from, to) = (a.center(), b.center());
    let go = match item {
        Item::Fetch => {
            // B asks first, then the clipboard comes across.
            let ask = ease(seg(t, 0.1, 0.35));
            let q = b.center().lerp(a.center(), ask);
            p.circle_filled(q, 3.0, mix(Color32::TRANSPARENT, INK2, (1.0 - seg(t, 0.35, 0.4)) * seg(t, 0.1, 0.15)));
            ease(seg(t, 0.4, 0.8))
        }
        // Marked sensitive: it starts to go, is turned back and stays.
        Item::Secret => ease(seg(t, 0.2, 0.4)) * (1.0 - ease(seg(t, 0.5, 0.7))) * 0.5,
        _ => ease(seg(t, 0.2, 0.7)),
    };
    let at = from.lerp(to, go) + vec2(0.0, -10.0 * (go * std::f32::consts::PI).sin());
    let alpha = 1.0 - seg(t, 0.92, 1.0);
    // The original stays where it was.
    glyph(p, from, item, 0.45);
    glyph(p, at, item, alpha);
    if item == Item::Secret && (0.4..0.7).contains(&t) {
        cross_mark(p, from.lerp(to, 0.5) + vec2(0.0, 6.0));
    }
}

fn glyph(p: &Painter, c: Pos2, item: Item, alpha: f32) {
    let ink = mix(Color32::TRANSPARENT, INK, alpha);
    let dim = mix(Color32::TRANSPARENT, INK2, alpha);
    let page = Rect::from_center_size(c, vec2(22.0, 26.0));
    p.rect_filled(page, CornerRadius::same(3), mix(Color32::TRANSPARENT, BUTTON_LIT, alpha));
    p.rect_stroke(page, CornerRadius::same(3), Stroke::new(1.0, dim), StrokeKind::Inside);
    let bar = |y: f32, w: f32, col: Color32, h: f32| p.rect_filled(Rect::from_min_size(pos2(page.left() + 4.0, page.top() + y), vec2(w, h)), CornerRadius::same(1), col);
    match item {
        Item::Image => {
            p.circle_filled(page.left_top() + vec2(8.0, 8.0), 2.5, mix(Color32::TRANSPARENT, SIGNAL, alpha));
            p.add(egui::Shape::convex_polygon(
                vec![page.left_bottom() + vec2(3.0, -3.0), page.left_bottom() + vec2(9.0, -12.0), page.left_bottom() + vec2(13.0, -7.0), page.left_bottom() + vec2(16.0, -10.0), page.right_bottom() + vec2(-3.0, -3.0)],
                dim,
                Stroke::NONE,
            ));
        }
        Item::Rich => {
            bar(5.0, 10.0, mix(Color32::TRANSPARENT, SIGNAL, alpha), 3.0);
            bar(11.0, 14.0, ink, 2.0);
            bar(16.0, 9.0, dim, 2.0);
        }
        Item::Text | Item::Fetch | Item::Secret => {
            bar(6.0, 14.0, ink, 2.0);
            bar(11.0, 14.0, dim, 2.0);
            bar(16.0, 9.0, dim, 2.0);
        }
    }
}

fn file(p: &Painter, c: Pos2, alpha: f32) {
    glyph(p, c, Item::Text, alpha);
}

fn mirror(p: &Painter, r: Rect, m: Both, t: f32) {
    match m {
        Both::Awake => {
            // Sleep tries to start and is turned away.
            let a = seg(t, 0.25, 0.4) * (1.0 - seg(t, 0.6, 0.75));
            moon(p, r.center(), a);
            if a > 0.2 {
                p.line_segment([r.center() + vec2(-11.0, 11.0), r.center() + vec2(11.0, -11.0)], Stroke::new(2.0, mix(Color32::TRANSPARENT, ERROR, a)));
            }
        }
        Both::Sleep => {
            let dim = seg(t, 0.3, 0.45) * (1.0 - seg(t, 0.8, 0.9));
            p.rect_filled(r.shrink(1.5), CornerRadius::same(5), mix(Color32::TRANSPARENT, GROUND, dim * 0.92));
            moon(p, r.center(), dim);
        }
        Both::Lock => {
            let on = seg(t, 0.35, 0.5) * (1.0 - seg(t, 0.85, 0.95));
            p.rect_filled(r.shrink(1.5), CornerRadius::same(5), mix(Color32::TRANSPARENT, GROUND, on * 0.85));
            padlock(p, r.center(), on);
        }
    }
}

fn moon(p: &Painter, c: Pos2, a: f32) {
    p.circle_filled(c, 8.0, mix(Color32::TRANSPARENT, INK, a));
    p.circle_filled(c + vec2(4.0, -3.0), 7.0, mix(Color32::TRANSPARENT, GROUND, a));
}

fn padlock(p: &Painter, c: Pos2, a: f32) {
    let col = mix(Color32::TRANSPARENT, INK, a);
    p.rect_filled(Rect::from_center_size(c + vec2(0.0, 3.0), vec2(16.0, 12.0)), CornerRadius::same(3), col);
    p.add(egui::Shape::line(vec![c + vec2(-5.0, -3.0), c + vec2(-5.0, -8.0), c + vec2(5.0, -8.0), c + vec2(5.0, -3.0)], Stroke::new(2.0, col)));
}

fn keycap(p: &Painter, c: Pos2, label: &str, pressed: f32) {
    let w = 12.0 + label.chars().count() as f32 * 6.4;
    let r = Rect::from_center_size(c + vec2(0.0, pressed), vec2(w, 20.0));
    p.rect_filled(r, CornerRadius::same(5), mix(BUTTON, SIGNAL, pressed));
    p.rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, KEY_EDGE), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, label, mono(T_META), mix(INK, SIGNAL_INK, pressed));
}

fn ring(p: &Painter, c: Pos2, r: f32, fill: f32) {
    let n = (fill * 24.0).ceil() as usize + 1;
    let pts = (0..n).map(|k| {
        let a = -std::f32::consts::FRAC_PI_2 + fill * std::f32::consts::TAU * k as f32 / (n - 1) as f32;
        c + vec2(a.cos(), a.sin()) * r
    });
    p.add(egui::Shape::line(pts.collect(), Stroke::new(2.0, SIGNAL)));
}

fn cross_mark(p: &Painter, c: Pos2) {
    let s = Stroke::new(2.0, ERROR);
    p.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], s);
    p.line_segment([c + vec2(-4.0, 4.0), c + vec2(4.0, -4.0)], s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_help_is_short_and_complete() {
        for (key, h) in HELP {
            assert!(!h.caption.is_empty() && h.caption.chars().count() <= CAPTION_MAX, "{key}: caption length {}", h.caption.chars().count());
            assert!(h.detail.len() > h.caption.len(), "{key}: the detail adds nothing to the caption");
        }
    }

    #[test]
    fn keys_are_unique() {
        for (k, (key, _)) in HELP.iter().enumerate() {
            assert!(HELP[k + 1..].iter().all(|(o, _)| o != key), "{key} twice");
        }
    }

    #[test]
    fn the_loop_wraps() {
        assert!(phase(0.0, 4.0) < 1e-6);
        assert!((phase(5.0, 4.0) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn settled_explainer_does_not_restart_after_idle() {
        assert_eq!(playback_start(60.0, 42, 0.0, 41), 0.0);
        assert_eq!(playback_start(120.0, 43, 0.0, 42), 0.0);
        assert_eq!(playback_start(121.0, 45, 0.0, 43), 121.0);
    }
}
