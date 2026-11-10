//! The Keys & mouse page: a drawn keyboard where a click and a key press
//! change what a key types on the other computers, quick setups, the rules
//! in sentences, a "Test it" box that runs the real key mapper here
//! (nothing is sent), and the mouse: its buttons, pointer speed and
//! scrolling. Changes go through `one-mk keymap|pointer|scroll|buttons`,
//! one at a time, off the window's thread; what is shown is read back from
//! `settings.toml`. The pure parts are in `crate::tune`.

use super::*;
use crate::tune::{self, Capture, Device, Facts, Input, KeyRule, Scope, ScrollChange, Tester};
use egui::{Event, EventFilter};
use one_mk_proto::keynames;
use one_mk_proto::remap::What;
use std::time::Instant;

const TEST_ID: &str = "keys-test";
const CAPTURE_ID: &str = "keys-capture";
const ROW: f32 = 36.0;
use crate::gear::{KEYBOARD_TURN as BOARD_TURN, MOUSE_TURN};

fn key_description(code: u16, action: Option<&What>) -> String {
    let name = tune::key_label(u32::from(code));
    match action {
        Some(What::To(output)) => format!("{name}: {}", tune::out_text(output)),
        Some(What::TapHold { tap, hold, .. }) => format!("{name}: {} when tapped, {} when held", tune::out_text(tap), tune::out_text(hold)),
        None => format!("{name}: unchanged"),
    }
}

/// The panel's When tapped and When held for a key that does `what` (the
/// same both ways without a hold; itself when unchanged).
fn tap_hold_text(what: Option<&What>, name: &str) -> (String, String) {
    match what {
        Some(What::To(o)) => (tune::out_text(o), tune::out_text(o)),
        Some(What::TapHold { tap, hold, .. }) => (tune::out_text(tap), tune::out_text(hold)),
        None => (name.to_owned(), name.to_owned()),
    }
}

fn key_matches_query(code: u16, query: &str) -> bool {
    tune::key_label(u32::from(code)).to_lowercase().contains(query)
        || keynames::key_name(u32::from(code)).to_lowercase().contains(query)
}

fn key_accessibility_activation(ui: &Ui, response: &egui::Response) -> bool {
    if !response.enabled() { return false; }
    ui.ctx().accesskit_node_builder(response.id, |node| node.add_action(egui::accesskit::Action::Click));
    ui.input(|input| input.has_accesskit_action_request(response.id, egui::accesskit::Action::Click))
}

fn gesture_args(button: &str, direction: &str, output: &str, scope: &Scope) -> Result<Vec<String>, String> {
    if !["middle", "button4", "button5"].contains(&button) || !["up", "down", "left", "right"].contains(&direction) {
        return Err("Choose a middle or side button and a swipe direction.".into());
    }
    output.parse::<one_mk_proto::remap::Output>().map_err(|_| "Choose a key or record a shortcut for this gesture.".to_owned())?;
    let mut args = vec!["gestures".into(), "add".into(), button.into(), direction.into(), output.into()];
    args.extend(scope.args());
    Ok(args)
}

#[cfg(test)]
mod gesture_tests {
    use super::*;

    #[test]
    fn gesture_commands_validate_and_preserve_scope() {
        assert_eq!(gesture_args("button4", "left", "ctrl+alt+k", &Scope::All).unwrap(), ["gestures", "add", "button4", "left", "ctrl+alt+k"]);
        let scope = Scope::Os(one_mk_proto::os::MACOS);
        assert!(gesture_args("middle", "up", "esc", &scope).unwrap().ends_with(&scope.args()));
        for (button, direction, output) in [("left", "up", "esc"), ("middle", "diagonal", "esc"), ("middle", "up", "ctrl+")] {
            assert!(gesture_args(button, direction, output, &Scope::All).is_err());
        }
    }

    #[test]
    fn a_rule_covers_the_computers_its_scope_names() {
        let peer = |name: &str, os| tune::Peer { name: name.into(), id: format!("{name}-id"), os };
        let (mac, linux) = (peer("Mac", one_mk_proto::os::MACOS), peer("Studio", one_mk_proto::os::LINUX));
        assert!(scope_covers(&Scope::All, &mac) && scope_covers(&Scope::All, &linux));
        assert!(scope_covers(&Scope::Os(one_mk_proto::os::LINUX), &linux) && !scope_covers(&Scope::Os(one_mk_proto::os::LINUX), &mac));
        assert!(scope_covers(&Scope::Peer("Studio".into()), &linux) && !scope_covers(&Scope::Peer("Studio".into()), &mac));
        // A keyboard narrows the rule to what is typed on it, not to a computer.
        let device = Scope::Device("aaaa-bbbb".into(), Box::new(Scope::Os(one_mk_proto::os::LINUX)));
        assert!(scope_covers(&device, &linux) && !scope_covers(&device, &mac));
    }

    /// The page as the shell gives it for a window `w` × `h`.
    fn page(w: f32, h: f32) -> Rect {
        Rect::from_min_max(pos2(shell::SIDE, shell::BAR_H + 4.0), pos2(w - shell::SIDE, h - M))
    }

    #[test]
    fn the_panel_follows_the_window_and_stacks_below_1000() {
        for (w, compact, panel) in [(820.0, true, 788.0), (999.0, true, 967.0), (1000.0, false, 310.0), (1279.0, false, 310.0), (1280.0, false, 350.0), (1699.0, false, 350.0), (1700.0, false, 380.0), (1920.0, false, 380.0)] {
            let lay = keys_layout(page(w, 900.0));
            assert_eq!(lay.compact, compact, "{w}");
            assert_eq!(lay.panel.width(), panel, "{w}");
            // 16 from the window's edges, 16 between the stage's room and the panel.
            assert_eq!(lay.panel.right(), w - 16.0);
            assert_eq!(lay.panel.bottom(), 900.0 - 16.0);
            if compact {
                assert_eq!((lay.panel.left(), lay.panel.height()), (16.0, 260.0));
                assert_eq!(lay.foot.height(), 0.0, "no quick setups row under the stage");
                assert!(lay.stage.bottom() <= lay.panel.top());
            } else {
                assert_eq!(lay.panel.top(), shell::BAR_H + 16.0);
                assert_eq!(lay.foot.height(), 30.0);
                assert!(lay.stage.right() <= lay.panel.left() - 16.0);
            }
        }
    }

    #[test]
    fn the_keyboard_scales_by_the_handoff_and_never_meets_the_header_or_the_setups() {
        assert_eq!(keyboard_scale(vec2(1130.0, 400.0)), 1.0);
        assert_eq!(keyboard_scale(vec2(2260.0, 800.0)), 1.0);
        assert_eq!(keyboard_scale(vec2(565.0, 400.0)), 0.5);
        assert_eq!(keyboard_scale(vec2(1130.0, 100.0)), 0.25);
        assert_eq!(keyboard_scale(vec2(100.0, 40.0)), 0.2);
        for (w, h) in [(820.0, 540.0), (999.0, 600.0), (1000.0, 600.0), (1100.0, 720.0), (1440.0, 900.0), (1920.0, 1200.0), (2560.0, 1440.0)] {
            let lay = keys_layout(page(w, h));
            assert!(lay.stage.top() >= lay.header.bottom() + 12.0, "{w}×{h}");
            assert!(lay.foot.top() >= lay.stage.bottom(), "{w}×{h}");
            let board = keyboard_rect(lay.stage);
            assert!(lay.stage.contains_rect(board), "the board leaves the stage at {w}×{h}: {board:?} in {:?}", lay.stage);
            assert!(board.top() > lay.header.bottom());
        }
        // 1440 × 900: as in the handoff's screen.
        let lay = keys_layout(page(1440.0, 900.0));
        assert_eq!(lay.header.min, pos2(28.0, 82.0));
        assert_eq!(lay.header.height(), 34.0);
        assert_eq!(lay.foot.bottom(), 880.0);
    }

    #[test]
    fn a_short_stage_keeps_the_whole_board_in_view() {
        assert!(!detail_room(keys_layout(page(820.0, 540.0)).stage));
        assert!(detail_room(keys_layout(page(1100.0, 720.0)).stage));
        assert!(detail_room(keys_layout(page(1440.0, 900.0)).stage));
    }

    #[test]
    fn the_panel_rows_say_what_a_key_does_tapped_and_held() {
        let tap_hold = What::TapHold { tap: "esc".parse().unwrap(), hold: "lctrl".parse().unwrap(), timeout: None };
        // The held key is named the way this system names it (Ctrl, or Control on a Mac).
        let ctrl = if cfg!(target_os = "macos") { "Control" } else { "Ctrl" };
        assert_eq!(tap_hold_text(Some(&tap_hold), "Caps Lock"), ("Esc".to_owned(), ctrl.to_owned()));
        let to = What::To("esc".parse().unwrap());
        assert_eq!(tap_hold_text(Some(&to), "Caps Lock"), ("Esc".to_owned(), "Esc".to_owned()));
        assert_eq!(tap_hold_text(None, "Caps Lock"), ("Caps Lock".to_owned(), "Caps Lock".to_owned()));
    }

    #[test]
    fn a_mouse_buttons_action_picks_its_choice() {
        let out = |text: &str| text.parse::<one_mk_proto::remap::Output>().unwrap();
        assert_eq!(ButtonKind::of(None), ButtonKind::AsItIs);
        assert_eq!(ButtonKind::of(Some(&out("switch next"))), ButtonKind::Switch);
        assert_eq!(ButtonKind::of(Some(&out("run: notify-send hi"))), ButtonKind::Run);
        assert_eq!(ButtonKind::of(Some(&out("back"))), ButtonKind::Key);
        assert_eq!(ButtonKind::of(Some(&out("ctrl+c"))), ButtonKind::Key);
        let labels: Vec<_> = ButtonKind::ALL.iter().map(|k| k.label()).collect();
        assert_eq!(labels, ["A key or shortcut", "Switch computer", "Run a command", "As it is"]);
    }

    #[test]
    fn key_description_uses_full_name_and_action() {
        assert_eq!(key_description(102, None), "Home: unchanged");
        let action = What::To("esc".parse().unwrap());
        assert_eq!(key_description(58, Some(&action)), "Caps Lock: Esc");
    }

    #[test]
    fn key_search_matches_display_and_canonical_names() {
        assert!(key_matches_query(71, "keypad 7"));
        assert!(key_matches_query(71, "kp7"));
        assert!(key_matches_query(102, "home"));
        assert!(!key_matches_query(102, "kp7"));
    }

    #[test]
    fn key_accessibility_action_activates_only_the_requested_cap() {
        let context = egui::Context::default();
        let selected = Id::unique("selected-cap");
        let other = Id::unique("other-cap");
        let input = egui::RawInput {
            events: vec![Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: selected.accesskit_id(),
                data: None,
            })],
            ..Default::default()
        };
        let mut output = context.run_ui(input, |ui| {
            let bounds = Rect::from_min_size(Pos2::ZERO, vec2(24.0, 24.0));
            let response = ui.interact(bounds, selected, Sense::focusable_noninteractive());
            assert!(key_accessibility_activation(ui, &response));
            let response = ui.interact(bounds.translate(vec2(28.0, 0.0)), other, Sense::hover());
            assert!(!key_accessibility_activation(ui, &response));
        });
        output.textures_delta.clear();
    }

    #[test]
    fn mouse_cards_sit_round_the_mouse_inside_the_stage() {
        for (w, h) in [(820.0, 540.0), (1000.0, 600.0), (1100.0, 720.0), (1440.0, 900.0), (1920.0, 1200.0)] {
            let stage = keys_layout(page(w, h)).stage;
            let cards = mouse_cards(stage);
            let room = mouse_room(stage);
            assert_eq!(cards.iter().map(|c| c.0).collect::<Vec<_>>(), [2, 3, 1, 4, 5], "right, wheel, left; back, forward");
            for (index, (button, card, left)) in cards.iter().enumerate() {
                assert_eq!(*left, *button <= 3);
                assert!(stage.expand(0.5).contains_rect(*card), "card {button} leaves the stage at {w}×{h}: {card:?} in {stage:?}");
                assert!(!card.intersects(room), "card {button} covers the mouse at {w}×{h}");
                assert!(card.width() <= 190.01 && [48.0, 34.0].iter().any(|h| (card.height() - h).abs() < 0.01), "{card:?} at {w}×{h}");
                for (_, other, _) in &cards[index + 1..] {
                    assert!(!card.intersects(*other), "cards overlap at {w}×{h}");
                }
            }
            assert!(room.width() >= 40.0 && room.height() >= 40.0);
        }
        // 1440 × 900: the handoff's cards, 190 × 48.
        let stage = keys_layout(page(1440.0, 900.0)).stage;
        assert!(mouse_cards(stage).iter().all(|c| (c.1.size() - vec2(190.0, 48.0)).length() < 0.01));
    }

    #[test]
    fn quick_setups_wrap_with_clear_rows_and_keep_the_heading_clear() {
        let context = egui::Context::default();
        crate::style::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            let presets = tune::presets(false);
            let heading_width = text_width(ui, "Quick setups", semibold(T_BODY)) + 16.0;
            for width in [420.0, 468.0, 628.0, 818.0, 1080.0] {
                for active in 0..1 << presets.len() {
                    let widths: Vec<_> = presets.iter().enumerate().map(|(index, preset)| chip_width(ui, &preset.label, active & (1 << index) != 0)).collect();
                    let (rects, bottom) = quick_setup_rects(width, heading_width, &widths);
                    for (index, rect) in rects.iter().enumerate() {
                        assert!(rect.left() >= 0.0 && rect.right() <= width);
                        assert!(rect.top() >= 38.0 || rect.left() >= heading_width);
                        assert!(rect.bottom() <= bottom);
                        for other in &rects[index + 1..] {
                            assert!(!rect.expand(3.9).intersects(*other), "setup rows collide at {width}");
                        }
                    }
                }
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn mouse_parts_stay_on_the_stage_through_orbit() {
        let model = crate::gear::Model::parse(include_bytes!("../../assets/gear/mouse.omkm")).unwrap();
        for (w, h) in [(1100.0, 720.0), (1440.0, 900.0)] {
            let stage = keys_layout(page(w, h)).stage;
            let room = mouse_room(stage);
            for yaw in [-0.9, 0.0, 0.9] {
                for pitch in [0.32, 0.62, 0.92] {
                    let mut view = None;
                    crate::gear::ModelView::get(&mut view, &model, stage, room, 0.0, -2.2 + yaw, pitch, None, None, &mut crate::gear::Glide::default(), 0.0);
                    for part in &view.unwrap().parts {
                        assert!(stage.contains(part.anchor), "{} leaves the stage at {w}×{h}, {yaw}, {pitch}", part.name);
                    }
                }
            }
        }
    }

    #[test]
    fn compact_setup_surface_is_scoped_to_keyboard() {
        for hover in [0.0, 0.5, 1.0] {
            assert_eq!(setup_fill(false, hover, false), mix(pal().surface, pal().surface2, hover));
            assert_eq!(setup_fill(true, hover, false), mix(pal().accent_soft, pal().pressed(pal().accent_soft, 0.4), hover));
            assert_ne!(setup_fill(false, hover, true), setup_fill(false, hover, false));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(super) enum View {
    #[default]
    Keyboard,
    Mouse,
}

/// What a key press or a choice from the list is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Want {
    /// What the selected key becomes (tapped, if it has a hold).
    To,
    /// What it becomes when held.
    Hold,
    /// What the selected mouse button does.
    Button,
}

/// The page's own state.
#[derive(Default)]
pub(super) struct KeysUi {
    pub view: View,
    /// "For:" which computers changes are made and shown for.
    pub scope: Scope,
    /// The keypad is drawn too ("More keys").
    pub keypad: bool,
    /// The key being changed, and the layer of the rule being changed.
    pub selected: Option<u16>,
    pub layer: Option<String>,
    /// The app the rule being changed is only for ("Only in app…").
    pub app: Option<String>,
    /// The key the keyboard's focus is on (arrows move it).
    pub focus_key: u16,
    /// Waiting for a key press.
    pub capture: Option<(Want, Capture)>,
    /// The list is open, with what was typed to search it.
    pub picker: Option<(Want, String)>,
    navigator_query: String,
    /// "Different when held" is open.
    pub more: bool,
    pub button: Option<u32>,
    /// An error and the area it belongs to (keyboard, list, buttons, pointer, scroll).
    pub error: Option<(&'static str, String)>,
    /// `one-mk keymap check`, and whether it should run again.
    pub problems: Option<Result<Vec<String>, String>>,
    pub check_due: bool,
    tester: Option<(Tester, Input, Facts)>,
    tester_error: Option<String>,
    test_focused: bool,
    n: u64,
    /// Values being dragged or just set, until settings.toml has them.
    pub drafts: HashMap<&'static str, f64>,
    pub device: Option<Device>,
    /// The keyboards and mice seen here, for "For:" (`None`: not asked yet).
    pub devices: Option<Result<tune::InputDevices, String>>,
    choices: Option<(bool, Vec<tune::Choice>)>,
    /// An editor just opened: bring it into view.
    reveal: bool,
    /// The 3D keyboard as last built, and the key the pointer is over.
    board: Option<crate::gear::Keyboard>,
    board_hover: Option<u16>,
    board_detail: bool,
    /// The stage has room to zoom in on a key ("Key detail").
    detail_ok: bool,
    /// The 3D mouse: its model (read once) and its view as last built.
    mouse_model: Option<std::sync::Arc<crate::gear::Model>>,
    mouse: Option<crate::gear::ModelView>,
    mouse_hover: Option<u32>,
    /// How far a drag has turned each, and where each one's camera is as it
    /// eases onto the chosen button or key.
    mouse_orbit: crate::gear::Orbit,
    board_orbit: crate::gear::Orbit,
    mouse_glide: crate::gear::Glide,
    board_glide: crate::gear::Glide,
    /// The mouse button's choice open in the panel ("A key or shortcut",
    /// "Switch computer", "Run a command"), for the button it was opened on.
    button_kind: Option<(u32, ButtonKind)>,
    /// The panel's contents as tall as they were last drawn.
    panel_heights: HashMap<&'static str, f32>,
}

/// What a mouse button does, as the panel's choices group it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ButtonKind {
    Key,
    Switch,
    Run,
    AsItIs,
}

impl ButtonKind {
    const ALL: [ButtonKind; 4] = [ButtonKind::Key, ButtonKind::Switch, ButtonKind::Run, ButtonKind::AsItIs];

    fn label(self) -> &'static str {
        match self {
            ButtonKind::Key => "A key or shortcut",
            ButtonKind::Switch => "Switch computer",
            ButtonKind::Run => "Run a command",
            ButtonKind::AsItIs => "As it is",
        }
    }

    /// The group a button's action is in (`None`: unchanged).
    fn of(what: Option<&one_mk_proto::remap::Output>) -> ButtonKind {
        use one_mk_proto::remap::{Action, Output};
        match what {
            None => ButtonKind::AsItIs,
            Some(Output::Action(Action::Switch(_))) => ButtonKind::Switch,
            Some(Output::Action(Action::Run(_))) => ButtonKind::Run,
            Some(_) => ButtonKind::Key,
        }
    }
}

/// Where the page's parts go (handoff 5.5 and 4.2): the header row, the
/// stage with the 3D keyboard or mouse, the row under it (quick setups, or
/// the mouse's hint), and the panel: at the right, or under the stage in a
/// compact window.
#[derive(Clone, Copy, Debug, PartialEq)]
struct KeysLayout {
    compact: bool,
    header: Rect,
    stage: Rect,
    foot: Rect,
    panel: Rect,
}

/// The panel's width for a window `width` wide (handoff 4.2).
fn panel_width(width: f32) -> f32 {
    if width >= 1700.0 { 380.0 } else if width >= 1280.0 { 350.0 } else { 310.0 }
}

/// `page` is the room under the top bar inside the window's 16-point sides
/// (as the shell gives a page): the window is 32 wider, its bar ends 4 above.
fn keys_layout(page: Rect) -> KeysLayout {
    let window_w = page.width() + 2.0 * shell::SIDE;
    let compact = window_w < shell::COMPACT_BELOW;
    let (top, bottom) = (page.top() - 4.0, page.bottom() + M);
    let (section, panel) = if compact {
        let panel = Rect::from_min_max(pos2(page.left(), bottom - 16.0 - 260.0), pos2(page.right(), bottom - 16.0));
        (Rect::from_min_max(pos2(page.left() - shell::SIDE, top), pos2(page.right() + shell::SIDE, panel.top())), panel)
    } else {
        let panel = Rect::from_min_max(pos2(page.right() - panel_width(window_w), top + 16.0), pos2(page.right(), bottom - 16.0));
        (Rect::from_min_max(pos2(page.left() - shell::SIDE, top), pos2(panel.left() - 16.0, bottom)), panel)
    };
    let header = Rect::from_min_max(pos2(section.left() + 28.0, top + 22.0), pos2(section.right() - 28.0, top + 22.0 + 34.0));
    let foot_h = if compact { 0.0 } else { 30.0 };
    let foot = Rect::from_min_max(pos2(header.left(), section.bottom() - 20.0 - foot_h), pos2(header.right(), section.bottom() - 20.0));
    let stage = Rect::from_min_max(pos2(header.left(), header.bottom() + 12.0), pos2(header.right(), (foot.top() - if compact { 0.0 } else { 12.0 }).max(header.bottom() + 12.0)));
    KeysLayout { compact, header, stage, foot, panel }
}

/// The keyboard's scale on a stage this big: `min(w/1130, h/400, 1)`, at least 0.2.
fn keyboard_scale(stage: egui::Vec2) -> f32 {
    (stage.x / 1130.0).min(stage.y / 400.0).clamp(0.2, 1.0)
}

/// The rectangle the keyboard's case is fitted into, centred on the stage
/// (the case without its shadow takes about 1050 × 372 of the 1130 × 400).
fn keyboard_rect(stage: Rect) -> Rect {
    let s = keyboard_scale(stage.size());
    Rect::from_center_size(stage.center(), vec2(1050.0 * s, 372.0 * s))
}

/// Whether the stage has room to zoom in on a key: on a short or narrow
/// one the whole board stays in view (the chosen key ringed).
fn detail_room(stage: Rect) -> bool {
    stage.width() >= 420.0 && stage.height() >= 200.0
}

/// The mouse's scale: `min(w/1010, h/420, 1.2)`, at least 0.2.
fn mouse_scale(stage: egui::Vec2) -> f32 {
    (stage.x / 1010.0).min(stage.y / 420.0).clamp(0.2, 1.2)
}

impl KeysUi {
    pub fn new() -> Self {
        KeysUi { check_due: true, focus_key: 58, keypad: true, ..Default::default() }
    }
}

fn mac() -> bool {
    cfg!(target_os = "macos")
}

/// A section of the panel (handoff 4.2: one surface, few borders): a
/// hairline above it, its title (12, `ink2`) under that, then its rows.
struct Card {
    top: Pos2,
    inner: Rect,
}

/// The gap between the panel's parts (handoff 4.2).
const PANEL_GAP: f32 = 18.0;

fn card_begin(ui: &Ui, top: Pos2, w: f32, title: &str) -> Card {
    let p = ui.painter();
    widgets::hairline(p, egui::Rangef::new(top.x, top.x + w), top.y);
    widgets::section_label(p, pos2(top.x, top.y + PANEL_GAP + 8.0), Align2::LEFT_CENTER, title, w);
    a11y_text(ui, Id::unique(("card", title)), Rect::from_min_size(top + vec2(0.0, PANEL_GAP), vec2(w, 16.0)), Role::Heading, title, None);
    Card { top, inner: Rect::from_min_max(top + vec2(0.0, PANEL_GAP + 22.0), pos2(top.x + w, f32::INFINITY)) }
}

impl Card {
    /// Closes the section below `bottom`; its height.
    fn end(self, _ui: &Ui, bottom: f32) -> f32 {
        bottom - self.top.y
    }
}

/// Buttons left to right, onto a new line when the row is full; returns
/// each one's rect and the bottom.
fn flow(ui: &Ui, left: f32, top: f32, width: f32, labels: &[&str]) -> (Vec<Rect>, f32) {
    let (mut x, mut y) = (left, top);
    let mut out = vec![];
    for l in labels {
        let w = button_width(ui, l) - 4.0;
        if x > left && x + w > left + width {
            x = left;
            y += 38.0;
        }
        out.push(Rect::from_min_size(pos2(x, y), vec2(w.min(width), 30.0)));
        x += w + 8.0;
    }
    (out, y + 30.0)
}

/// A chip that is on or off: a quick setup (the handoff's chip: on, the
/// accent's edge on `accent_soft`; off, `surface` in a `line2` edge). In
/// the compact panel an off chip is a quiet fill.
fn setup_fill(on: bool, hover: f32, compact: bool) -> Color32 {
    match (compact, on) {
        (_, true) => mix(pal().accent_soft, pal().pressed(pal().accent_soft, 0.4), hover),
        // In the panel (on `surface`), a quiet fill; on the stage's floor,
        // a `surface` chip that lifts off it.
        (true, false) => mix(pal().surface2, pal().surface2_hover, hover),
        (false, false) => mix(pal().surface, pal().surface2, hover),
    }
}

fn chip(ui: &mut Ui, rect: Rect, id: Id, label: &str, on: bool, enabled: bool, compact: bool) -> egui::Response {
    let r = ui.interact(widgets::hit_rect(rect), id, if enabled { widgets::sense(ui, Sense::click()) } else { Sense::hover() });
    let hover = tween(ui.ctx(), id.with("hover"), f32::from(enabled && r.hovered()), 0.12, motion::out_cubic);
    let p = ui.painter();
    let radius = CornerRadius::same((rect.height() / 2.0) as u8);
    // The handoff's chip: 30 tall, a `line2` edge; on, the accent's edge on
    // `accent_soft` with `accent_text`. The words (500 12) sit centred.
    let press = tween(ui.ctx(), id.with("press"), f32::from(enabled && r.is_pointer_button_down_on()), 0.08, motion::out_cubic);
    let fill = setup_fill(on, hover, compact);
    let surface = if press > 0.0 { mix(fill, pal().pressed(if on { pal().accent_soft } else { pal().surface2 }, 1.0), press) } else { fill };
    let ink = if !enabled { pal().ink3 } else if on { pal().accent_text } else { pal().ink };
    p.rect_filled(rect, radius, surface);
    p.rect_stroke(rect, radius, Stroke::new(1.0, if on { pal().accent } else { pal().line2 }), StrokeKind::Inside);
    let font = medium(T_META);
    let text = fit(p, label, &font, rect.width() - 20.0);
    p.text(rect.center(), Align2::CENTER_CENTER, text, font, ink);
    widgets::focus_ring(ui, &r, rect, radius.nw);
    if r.hovered() && enabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    widgets::targets::note(label, rect);
    let (name, enabled) = (label.to_owned(), enabled && r.enabled());
    r.widget_info(move || egui::WidgetInfo::selected(Role::CheckBox, enabled, on, &name));
    r
}

fn chip_width(ui: &Ui, label: &str, _on: bool) -> f32 {
    text_width(ui, label, medium(T_META)) + 26.0
}

fn quick_setup_rects(width: f32, heading_width: f32, widths: &[f32]) -> (Vec<Rect>, f32) {
    let widest = widths.iter().copied().fold(0.0, f32::max);
    let left = if heading_width + widest <= width { heading_width } else { 0.0 };
    let (mut x, mut y) = (left, if left == 0.0 { 38.0 } else { 0.0 });
    let mut rects = Vec::with_capacity(widths.len());
    for &chip_width in widths {
        let chip_width = chip_width.min(width - left);
        if x > left && x + chip_width > width {
            x = left;
            y += 38.0;
        }
        rects.push(Rect::from_min_size(pos2(x, y), vec2(chip_width, 30.0)));
        x += chip_width + 8.0;
    }
    (rects, y + 30.0)
}

/// The mouse's labels (handoff 5.5): cards 190 × 48 on leader lines, the
/// right button, the wheel and the left button down the left, the side
/// buttons down the right, where the handoff's 1000 × 420 scene puts them;
/// on a short stage they become one line (34) and close up. They never
/// move as the mouse turns, so neither the labels nor their targets jump.
/// Each: the button, its card, and whether it is on the left.
fn mouse_cards(stage: Rect) -> Vec<(u32, Rect, bool)> {
    let s = mouse_scale(stage.size());
    let scene = Rect::from_center_size(stage.center(), vec2(1000.0, 420.0) * s);
    let width = 190.0_f32.min(((stage.width() - 160.0) / 2.0).max(96.0));
    let height = if stage.height() >= 3.0 * 48.0 + 2.0 * 10.0 + 8.0 { 48.0 } else { 34.0 };
    let mut out = Vec::with_capacity(5);
    for (left, buttons, tops) in [(true, &[2u32, 3, 1][..], &[114.0, 199.0, 284.0][..]), (false, &[4, 5][..], &[159.0, 229.0][..])] {
        // The handoff's spacing at this scale, or closer when the cards would touch.
        let step = ((tops[1] - tops[0]) * s).max(height + 8.0).min((stage.height() - height) / (buttons.len() - 1) as f32);
        let column = step * (buttons.len() - 1) as f32 + height;
        let centre = scene.top() + (tops[0] + tops[tops.len() - 1] + 48.0) / 2.0 * s;
        let top = (centre - column / 2.0).clamp(stage.top(), (stage.bottom() - column).max(stage.top()));
        // Where the handoff puts them, but never over the mouse (608 wide in
        // the scene), and inside the stage.
        let half = 304.0 * s;
        let x = if left {
            (scene.left() + 6.0 * s).min(stage.center().x - half - width).max(stage.left())
        } else {
            (scene.left() + 804.0 * s).max(stage.center().x + half).min(stage.right() - width)
        };
        for (k, button) in buttons.iter().enumerate() {
            out.push((*button, Rect::from_min_size(pos2(x, top + step * k as f32), vec2(width, height)), left));
        }
    }
    out
}

/// Where the mouse itself is framed: between the label columns.
fn mouse_room(stage: Rect) -> Rect {
    let cards = mouse_cards(stage);
    let left = cards.iter().filter(|c| c.2).map(|c| c.1.right()).fold(stage.left(), f32::max) + 8.0;
    let right = cards.iter().filter(|c| !c.2).map(|c| c.1.left()).fold(stage.right(), f32::min) - 8.0;
    let s = mouse_scale(stage.size());
    let scene = Rect::from_center_size(stage.center(), vec2(1000.0, 420.0) * s);
    let (top, bottom) = ((scene.top() + 40.0 * s).max(stage.top() + 4.0), (scene.top() + 380.0 * s).min(stage.bottom() - 4.0));
    Rect::from_min_max(pos2(left, top), pos2(right.max(left + 40.0), bottom.max(top + 40.0)))
}

/// A slider from 0 to 1: dragged, clicked, or moved with the arrows when
/// focused. Returns the new position and whether to apply it now (let go,
/// or a step with the keyboard).
fn slider(ui: &mut Ui, rect: Rect, id: Id, t: f32, name: &str, value: f64) -> (Option<f32>, bool) {
    let r = ui.interact(widgets::hit_rect(rect), id, widgets::sense(ui, Sense::click_and_drag()));
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 18.0, 4.0));
    let mut out = None;
    let mut apply = false;
    if (r.dragged() || r.clicked()) && let Some(p) = r.interact_pointer_pos() {
        out = Some(((p.x - track.left()) / track.width()).clamp(0.0, 1.0));
        apply = r.clicked();
    }
    if r.drag_stopped() {
        apply = true;
        out = out.or(Some(t));
    }
    if r.has_focus() {
        ui.memory_mut(|m| m.set_focus_lock_filter(id, EventFilter { horizontal_arrows: true, ..Default::default() }));
        let step = ui.input(|i| f32::from(i.key_pressed(Key::ArrowRight)) - f32::from(i.key_pressed(Key::ArrowLeft)));
        if step != 0.0 {
            out = Some((t + step / 46.0).clamp(0.0, 1.0));
            apply = true;
        }
    }
    let shown = out.unwrap_or(t);
    let p = ui.painter();
    p.rect_filled(track, CornerRadius::same(2), pal().surface2);
    let x = track.left() + track.width() * shown;
    p.rect_filled(Rect::from_min_max(track.min, pos2(x, track.bottom())), CornerRadius::same(2), pal().ink2);
    // The middle is "as it is" (1×).
    p.line_segment([pos2(track.center().x, track.top() - 5.0), pos2(track.center().x, track.top() - 2.0)], Stroke::new(1.0, pal().key_edge));
    let knob = pos2(x, track.center().y);
    // Held, the knob dims a step: pressed, apart from hover and focus.
    p.circle_filled(knob, 8.0, if r.is_pointer_button_down_on() || r.dragged() { pal().ink2 } else if r.hovered() { pal().tint_ink } else { pal().ink });
    if r.has_focus() && widgets::keyboard_focus(ui.ctx()) {
        p.circle_stroke(knob, 11.0, Stroke::new(2.0, pal().focus));
    }
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let (name, enabled) = (name.to_owned(), r.enabled());
    r.widget_info(move || egui::WidgetInfo::slider(enabled, value, &name));
    (out, apply)
}

/// Draws a pencil: edit.
fn draw_pencil(p: &Painter, rect: Rect, col: Color32) {
    let c = rect.center();
    let st = Stroke::new(1.4, col);
    p.line_segment([c + vec2(-4.5, 4.5), c + vec2(3.5, -3.5)], st);
    p.line_segment([c + vec2(-2.5, 5.5), c + vec2(5.0, -2.0)], st);
    p.line_segment([c + vec2(3.5, -3.5), c + vec2(5.0, -2.0)], st);
    p.line_segment([c + vec2(-4.5, 4.5), c + vec2(-5.5, 6.0)], st);
}

/// Draws a cross: remove, close.
fn draw_cross(p: &Painter, rect: Rect, col: Color32) {
    let c = rect.center();
    let st = Stroke::new(1.5, col);
    p.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], st);
    p.line_segment([c + vec2(-4.0, 4.0), c + vec2(4.0, -4.0)], st);
}

/// The floor under the 3D objects (handoff 3): a soft radial ellipse of
/// the theme's `floor`, 820 × 120 at scale 1, 130 below the stage's middle.
fn floor(p: &Painter, stage: Rect, scale: f32) {
    let c = pos2(stage.center().x, stage.center().y + 130.0 * scale);
    let (rx, ry) = (410.0 * scale, 60.0 * scale);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, pal().floor);
    // As the handoff's gradient: from `floor` at the middle to clear at 70 %.
    let rings = [(0.35, 0.55), (0.7, 0.0)];
    const N: u32 = 48;
    for (k, (at, alpha)) in rings.iter().enumerate() {
        for i in 0..N {
            let a = i as f32 / N as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(c + vec2(a.cos() * rx * at, a.sin() * ry * at), pal().floor.gamma_multiply(*alpha));
        }
        let base = 1 + k as u32 * N;
        for i in 0..N {
            let j = (i + 1) % N;
            if k == 0 {
                mesh.add_triangle(0, base + i, base + j);
            } else {
                let inner = base - N;
                mesh.add_triangle(inner + i, base + i, base + j);
                mesh.add_triangle(inner + i, base + j, inner + j);
            }
        }
    }
    p.add(egui::Shape::mesh(mesh));
}

/// The ground's colour fading in over `width` points at each edge of `r`.
fn edge_fade(p: &Painter, r: Rect, width: f32) {
    let (solid, clear) = (pal().bg, Color32::TRANSPARENT);
    let mut mesh = egui::Mesh::default();
    let mut quad = |a: Rect, cols: [Color32; 4]| {
        let i = mesh.vertices.len() as u32;
        for (pos, col) in [a.left_top(), a.right_top(), a.right_bottom(), a.left_bottom()].into_iter().zip(cols) {
            mesh.colored_vertex(pos, col);
        }
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    };
    quad(Rect::from_min_max(r.min, pos2(r.left() + width, r.bottom())), [solid, clear, clear, solid]);
    quad(Rect::from_min_max(pos2(r.right() - width, r.top()), r.max), [clear, solid, solid, clear]);
    quad(Rect::from_min_max(r.min, pos2(r.right(), r.top() + width)), [solid, solid, clear, clear]);
    quad(Rect::from_min_max(pos2(r.left(), r.bottom() - width), r.max), [clear, clear, solid, solid]);
    p.add(egui::Shape::mesh(mesh));
}

/// A small quiet button (a toolbar's, a menu's): `surface2`, its label in
/// 500 12, exactly `rect` (the label is fitted, not the button widened).
fn small_button(ui: &mut Ui, rect: Rect, id: Id, label: &str, a11y_label: Option<&str>) -> egui::Response {
    let r = ui.interact(widgets::hit_rect(rect), id, widgets::sense(ui, Sense::click()));
    let hover = tween(ui.ctx(), id.with("hover"), f32::from(r.hovered()), 0.1, motion::out_cubic);
    let press = tween(ui.ctx(), id.with("press"), f32::from(r.is_pointer_button_down_on()), 0.08, motion::out_cubic);
    let radius = CornerRadius::same(widgets::button_radius(rect.height()));
    let p = ui.painter();
    // It floats on the stage's floor: `surface` in a card's edge on a
    // 1-point contact shadow (a quiet fill on the ground reads muddy).
    let lift = if pal().theme == crate::style::Theme::Light { 0.06 } else { 0.4 };
    p.add(egui::epaint::Shadow { offset: [0, 1], blur: 2, spread: 0, color: pal().shadow_ink.gamma_multiply(lift * (1.0 - press)) }.as_shape(rect, radius));
    p.rect(rect, radius, widgets::pressed(mix(pal().surface, pal().surface2_hover, hover), press), Stroke::new(1.0, pal().card_edge), StrokeKind::Inside);
    if pal().theme == crate::style::Theme::Dark {
        p.add(widgets::top_light(rect, radius.nw));
    }
    if !label.is_empty() {
        p.text(rect.center(), Align2::CENTER_CENTER, fit(p, label, &medium(T_META), rect.width() - 12.0), medium(T_META), pal().ink);
    }
    widgets::focus_ring(ui, &r, rect, radius.nw);
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    widgets::a11y(&r, Role::Button, a11y_label.unwrap_or(label), None);
    r
}

/// A key or shortcut in mono on a `surface2` chip (padding 4 × 8, corner
/// 6), at most `max_w` wide; its rectangle.
fn kbd_fit(p: &Painter, at: Pos2, anchor: Align2, text: &str, max_w: f32) -> Rect {
    let shown = fit(p, text, &mono(T_META), (max_w - 16.0).max(10.0));
    let g = p.layout_no_wrap(shown, mono(T_META), pal().ink);
    let rect = anchor.anchor_size(at, g.size() + vec2(16.0, 8.0));
    p.rect(rect, CornerRadius::same(6), pal().surface2, Stroke::new(1.0, pal().line), StrokeKind::Inside);
    p.galley(rect.center() - g.size() / 2.0, g, pal().ink);
    rect
}

/// An error line: a red dot and the message, wrapped. Its height.
fn error_line(ui: &Ui, left: f32, y: f32, width: f32, text: &str) -> f32 {
    ui.painter().circle_filled(pos2(left + 3.0, y + 9.0), 3.0, pal().danger);
    let h = widgets::para_code(ui, pos2(left + 14.0, y), width - 14.0, text, regular(T_BODY), pal().ink);
    a11y_text(ui, Id::unique(("keys-error", text)), Rect::from_min_size(pos2(left, y), vec2(width, h)), Role::Alert, text, Some(Live::Assertive));
    h
}

impl App {
    /// The page takes the keyboard (a key being pressed for a rule, or the
    /// test box): the window's own shortcuts wait.
    pub(super) fn keys_grab(&self, ctx: &egui::Context) -> bool {
        self.tab == Tab::Keys && (self.keys.capture.is_some() || ctx.memory(|m| m.has_focus(Id::unique(TEST_ID))))
    }

    fn scope_peers(&self) -> Vec<tune::Peer> {
        self.running().map_or(vec![], |s| s.peers.iter().map(|p| tune::Peer { name: p.name.clone(), id: p.id.clone(), os: p.os }).collect())
    }

    fn facts(&self) -> Facts {
        Facts::of(&self.keys.scope, &self.scope_peers())
    }

    fn tuning_busy(&self) -> bool {
        self.busy.iter().any(|j| matches!(j, Job::Tune { .. }))
    }

    /// Runs `cmds` in order through `one-mk`; an error shows in `area`.
    fn tune_cmds(&mut self, what: &str, area: &'static str, cmds: Vec<Vec<String>>, key: Option<String>) {
        if cmds.is_empty() {
            return;
        }
        self.keys.n += 1;
        let job = Job::Tune { what: what.into(), area, n: self.keys.n };
        self.busy.push(job.clone());
        if self.keys.error.as_ref().is_some_and(|(a, _)| *a == area) {
            self.keys.error = None;
        }
        self.link.tune(job, cmds, key);
    }

    /// A tuning command finished.
    pub(super) fn tuned(&mut self, area: &'static str, what: &str, error: Option<String>) {
        self.keys.check_due = true;
        if let Some(e) = error {
            self.keys.drafts.clear();
            self.prefs = self.link.prefs();
            self.keys.error = Some((area, format!("Couldn't change {what}: {e}")));
        }
    }

    /// The rule for a key in the chosen scope and layer, if there is one.
    fn own_rule(&self, code: u16) -> Option<(usize, KeyRule)> {
        let name = keynames::key_name(u32::from(code));
        let want = KeyRule { scope: self.keys.scope.clone(), layer: self.keys.layer.clone(), ..KeyRule::to(&name, "none") };
        let i = self.prefs.input.keys.iter().position(|r| {
            r.scope == want.scope
                && tune::same_app(&r.app, &self.keys.app)
                && r.layer.as_ref().map(|l| l.to_ascii_lowercase()) == want.layer.as_ref().map(|l| l.to_ascii_lowercase())
                && r.from.parse::<keynames::Trigger>().is_ok_and(|t| t == keynames::Trigger::key(code))
        })?;
        Some((i, self.prefs.input.keys[i].clone()))
    }

    /// What `value` (a key, chord or action, as written) does for the
    /// key or button being changed: checked, then saved.
    pub(super) fn choose(&mut self, want: Want, value: String) {
        self.keys.capture = None;
        self.keys.picker = None;
        if want == Want::Button {
            let Some(n) = self.keys.button else { return };
            if let Err(e) = tune::check_button(n, &value) {
                self.keys.error = Some(("buttons", e));
                return;
            }
            let name = tune::BUTTONS.iter().find(|b| b.0 == n).map_or("the button", |b| b.1);
            self.tune_cmds(&name.to_lowercase(), "buttons", vec![tune::button_args(n, Some(&value), &self.keys.scope)], None);
            return;
        }
        let Some(code) = self.keys.selected else { return };
        let from = keynames::key_name(u32::from(code));
        let own = self.own_rule(code).map(|(_, r)| r);
        let base = KeyRule { scope: self.keys.scope.clone(), layer: self.keys.layer.clone(), app: self.keys.app.clone(), ..Default::default() };
        let tap_of = |r: &KeyRule| r.tap.clone().or_else(|| match &r.to {
            Some(tune::To::One(s)) => Some(s.clone()),
            _ => None,
        });
        let rule = match (want, own.as_ref().and_then(|r| r.hold.clone())) {
            (Want::To, Some(hold)) => KeyRule { tap: Some(value), hold: Some(hold), timeout_ms: own.as_ref().and_then(|r| r.timeout_ms), from, ..base },
            (Want::Hold, _) => {
                let tap = own.as_ref().and_then(tap_of).unwrap_or_else(|| from.clone());
                KeyRule { tap: Some(tap), hold: Some(value), timeout_ms: own.as_ref().and_then(|r| r.timeout_ms), from, ..base }
            }
            _ => KeyRule { to: Some(tune::To::One(value)), from, ..base },
        };
        self.save_rule(rule);
    }

    fn save_rule(&mut self, rule: KeyRule) {
        if let Err(e) = tune::check_add(&self.prefs.input, &rule) {
            self.keys.error = Some(("keyboard", e));
            return;
        }
        let what = rule.from.parse::<keynames::Trigger>().map_or(rule.from.clone(), |t| tune::trigger_text(&t));
        self.tune_cmds(&what, "keyboard", vec![tune::add_args(&rule)], None);
    }

    pub(super) fn select_key(&mut self, code: u16, layer: Option<String>) {
        self.keys.selected = Some(code);
        self.keys.focus_key = code;
        self.keys.layer = layer;
        self.keys.board_detail = self.keys.layer.is_none();
        self.keys.app = None;
        self.keys.capture = None;
        self.keys.picker = None;
        // The When held row says what it gives; its choices open on a click.
        self.keys.more = false;
        self.keys.reveal = true;
        if self.keys.error.as_ref().is_some_and(|(a, _)| *a == "keyboard") {
            self.keys.error = None;
        }
    }

    /// Development aid: types `spec` into the test box (`58d0,58u85`: a
    /// code, down or up, at a time in ms), as if it had just happened.
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    pub(super) fn debug_type(&mut self, ctx: &egui::Context, spec: &str) {
        ctx.memory_mut(|m| m.request_focus(Id::unique(TEST_ID)));
        self.keys.test_focused = true;
        let steps: Vec<(u16, bool, u64)> = spec
            .split(',')
            .filter_map(|s| {
                let s = s.trim();
                let at = s.find(['d', 'u'])?;
                Some((s[..at].parse().ok()?, &s[at..=at] == "d", s[at + 1..].parse().ok()?))
            })
            .collect();
        let end = steps.iter().map(|s| s.2).max().unwrap_or(0);
        let base = Instant::now() - std::time::Duration::from_millis(end);
        if let Some(t) = self.tester() {
            for (code, down, at) in steps {
                let when = base + std::time::Duration::from_millis(at);
                t.tick(when);
                t.key(code, down, when);
            }
        }
    }

    /// The rules compiled for the chosen computers, for the test box.
    fn tester(&mut self) -> Option<&mut Tester> {
        let facts = self.facts();
        let input = &self.prefs.input;
        let fresh = !self.keys.tester.as_ref().is_some_and(|(_, i, f)| i == input && *f == facts);
        if fresh {
            match input.keymap(&facts) {
                Ok(map) => {
                    self.keys.tester_error = None;
                    match &mut self.keys.tester {
                        Some((t, i, f)) => {
                            t.set_keymap(map, input.tap());
                            (*i, *f) = (input.clone(), facts);
                        }
                        None => self.keys.tester = Some((Tester::new(map, input.tap()), input.clone(), facts)),
                    }
                }
                Err(e) => {
                    self.keys.tester_error = Some(e);
                    self.keys.tester = None;
                }
            }
        }
        self.keys.tester.as_mut().map(|(t, ..)| t)
    }

    /// Key events for a key being pressed for a rule, or for the test box.
    fn keys_input(&mut self, ui: &mut Ui) {
        let take = |ui: &mut Ui, tab_too: bool| -> Vec<Event> {
            ui.input_mut(|i| {
                let (taken, kept): (Vec<Event>, Vec<Event>) = std::mem::take(&mut i.events).into_iter().partition(|e| match e {
                    Event::Key { key: Key::Tab, .. } => tab_too,
                    Event::Key { .. } | Event::Copy | Event::Cut | Event::Paste(_) | Event::Text(_) => true,
                    _ => false,
                });
                i.events = kept;
                taken
            })
        };
        if let Some((want, mut cap)) = self.keys.capture {
            let id = Id::unique(CAPTURE_ID);
            ui.memory_mut(|m| {
                m.request_focus(id);
                m.set_focus_lock_filter(id, EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true });
            });
            for e in take(ui, true) {
                let Some((code, down, repeat)) = tune::event_key(&e) else { continue };
                if repeat {
                    continue;
                }
                if code == 1 && down && !cap.holding() {
                    self.keys.capture = None;
                    return;
                }
                if let Some(chord) = cap.key(code, down) {
                    self.choose(want, chord.to_string());
                    return;
                }
            }
            self.keys.capture = Some((want, cap));
            return;
        }
        let id = Id::unique(TEST_ID);
        let focused = ui.memory(|m| m.has_focus(id));
        if self.keys.test_focused && !focused && let Some((t, ..)) = &mut self.keys.tester {
            t.release();
        }
        self.keys.test_focused = focused;
        if !focused {
            return;
        }
        ui.memory_mut(|m| m.set_focus_lock_filter(id, EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: true }));
        let events = take(ui, false);
        let now = Instant::now();
        if let Some(t) = self.tester() {
            for e in &events {
                if let Some((code, down, _)) = tune::event_key(e) {
                    t.key(code, down, now);
                }
            }
        }
    }

    // ---------- the page ----------

    /// Keys & mouse (handoff 5.5): the header row, the 3D keyboard or mouse
    /// on the floor, and the panel. The stage keeps its place and size
    /// whatever is chosen, so the camera's glide onto a key is never cut short.
    pub(super) fn keys_page(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        if self.keys.focus_key == 0 {
            self.keys.focus_key = 58;
        }
        if self.keys.check_due && !self.tuning_busy() {
            self.keys.check_due = false;
            self.link.keymap_check();
        }
        self.keys_input(ui);
        // A tap/hold key in the test box decides when its time is up.
        if let Some((t, ..)) = &mut self.keys.tester
            && let Some(d) = t.deadline()
        {
            let i = Instant::now();
            if i >= d {
                t.tick(i);
            } else if motion::reduced() {
                ui.ctx().request_repaint_after(d - i);
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(33).min(d - i));
            }
        }
        let lay = keys_layout(rect);
        self.keys_header(ui, lay.header, now);
        let input = self.prefs.input.clone();
        let facts = self.facts();
        match self.keys.view {
            View::Keyboard => {
                self.keyboard_stage(ui, lay.stage, lay.compact, &input, &facts);
                if !lay.compact {
                    self.quick_setups_row(ui, lay.foot, &input);
                }
            }
            View::Mouse => {
                self.mouse_stage(ui, lay.stage, &input, &facts);
                if !lay.compact {
                    let hint = "Click a button on the mouse or its label";
                    ui.painter().text(lay.foot.center(), Align2::CENTER_CENTER, hint, regular(T_META), pal().ink3);
                }
            }
        }
        self.keys_panel(ui, lay.panel, lay.compact, now);
    }

    /// Title (22/600), Keyboard | Mouse, and "For: All computers ▾" at the right.
    fn keys_header(&mut self, ui: &mut Ui, r: Rect, now: f64) {
        let title = "Keys & mouse";
        let tw = widgets::title_text(ui.painter(), r.left_center(), Align2::LEFT_CENTER, title, T_SECTION, r.width()).width();
        a11y_text(ui, Id::unique(("page", title)), Rect::from_min_size(r.min, vec2(tw, r.height())), Role::Heading, title, None);
        let labels = ["Keyboard", "Mouse"];
        let widths: Vec<f32> = labels.iter().map(|l| text_width(ui, l, semibold(T_BODY)) + 28.0).collect();
        let track = Rect::from_min_size(pos2(r.left() + tw + 18.0, r.center().y - 17.0), vec2(widths.iter().sum::<f32>() + 8.0, 34.0));
        let mut x = track.left() + 3.0;
        let cells = widths.iter().map(|w| {
            let cell = Rect::from_min_size(pos2(x, track.top() + 3.0), vec2(*w, 28.0));
            x += w + 2.0;
            cell
        }).collect();
        let pills = Pills {
            id: Id::unique("keys-view"),
            track,
            cells,
            labels: &labels,
            names: vec!["Keyboard: change keys".into(), "Mouse: buttons, pointer and scrolling".into()],
            chosen: Some(usize::from(self.keys.view == View::Mouse)),
            enabled: vec![true; 2],
            role: Role::Tab,
            font: T_BODY,
        };
        if let Some(k) = pills.show(ui, self.arrowed) {
            self.keys.view = if k == 0 { View::Keyboard } else { View::Mouse };
            self.keys.capture = None;
            self.keys.picker = None;
        }
        let room = Rect::from_min_max(pos2(track.right() + 16.0, r.center().y - 16.0), pos2(r.right(), r.center().y + 16.0));
        let scope = self.scope_menu(ui, room);
        if self.tuning_busy() {
            let p = ui.painter();
            let label_w = text_width(ui, "Saving…", regular(T_META));
            let x = (scope.left() - 12.0 - label_w - 22.0).max(track.right() + 12.0);
            widgets::draw_refresh(p, Rect::from_center_size(pos2(x + 8.0, r.center().y), vec2(16.0, 16.0)), pal().ink2, if motion::reduced() { 0.0 } else { (now * 5.0) as f32 });
            p.text(pos2(x + 22.0, r.center().y), Align2::LEFT_CENTER, "Saving…", regular(T_META), pal().ink2);
            ui.ctx().request_repaint_after_secs(1.0 / 30.0);
        }
    }

    /// The panel (handoff 4.2): `surface` at the right, or under the stage
    /// (260 tall) in a compact window; what it holds scrolls.
    fn keys_panel(&mut self, ui: &mut Ui, r: Rect, compact: bool, now: f64) {
        widgets::panel(ui, r);
        let inner = r.shrink(20.0);
        // A little room round the contents for focus rings, and on the
        // right for the scroll bar (inside the panel's padding).
        let view = Rect::from_min_max(inner.min - vec2(6.0, 6.0), inner.max + vec2(14.0, 6.0));
        let mut child = ui.new_child(UiBuilder::new().max_rect(view).id_salt("keys-panel"));
        child.set_clip_rect(view.intersect(ui.clip_rect()));
        if motion::reduced() {
            child.style_mut().scroll_animation = egui::style::ScrollAnimation::none();
        }
        let key = if self.keys.view == View::Keyboard { "keyboard" } else { "mouse" };
        let scroll = egui::ScrollArea::vertical().id_salt(("keys-panel", key)).auto_shrink([false, false]);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        let scroll = std::env::var("ONE_MK_APP_SCROLL").ok().and_then(|value| value.parse::<f32>().ok()).map_or(scroll.clone(), |offset| scroll.vertical_scroll_offset(offset));
        scroll.show(&mut child, |ui| {
            let viewport = ui.clip_rect().height() - 12.0;
            let total = self.keys.panel_heights.get(key).copied().unwrap_or(viewport).max(viewport);
            let (all, _) = ui.allocate_exact_size(vec2(view.width() - 8.0, total + 12.0), Sense::hover());
            let at = all.min + vec2(6.0, 6.0);
            let w = inner.width();
            let mut used = match self.keys.view {
                View::Keyboard => self.keyboard_panel(ui, at, w, viewport, compact, now),
                View::Mouse => self.mouse_panel(ui, at, w),
            };
            // Only where it can work: on Linux there is nothing to switch.
            if cfg!(any(windows, target_os = "macos")) {
                used += PANEL_GAP + self.local_rules_card(ui, at + vec2(0.0, used + PANEL_GAP), w);
            }
            if (self.keys.panel_heights.get(key).copied().unwrap_or(0.0) - used).abs() > 0.5 {
                self.keys.panel_heights.insert(key, used);
                ui.ctx().request_repaint();
            }
        });
    }

    fn local_rules_card(&mut self, ui: &mut Ui, at: Pos2, width: f32) -> f32 {
        let available = cfg!(any(windows, target_os = "macos"));
        let input = self.prefs.input.clone();
        let card = card_begin(ui, at, width, "Where your rules work");
        let content = card.inner;
        let row = Rect::from_min_size(content.min, vec2(content.width(), ROW));
        if widgets::switch_help_dimmed(ui, row, Id::unique("also-on-this-computer"), "Also on this computer (experimental)", "also-on-this-computer", input.local_rules,
            available && !self.tuning_busy() && !self.prefs.unreadable).clicked() {
            self.tune_cmds("local rules", "local-rules", vec![vec!["keymap".into(), "local".into(),
                if input.local_rules { "off".into() } else { "on".into() }]], None);
        }
        let text = if available {
            "Experimental, not yet tested on real hardware: apply your key and button rules here as well as on the other computers. Pointer speed and scrolling stay unchanged. Ctrl + Alt + Shift + Esc turns local rules off and brings you back here. Local rules keep working when this window closes. Stopping oneMK restores the original mappings; your preference is kept for the next start."
        } else {
            "Not available on this desktop. Local rules need macOS or Windows; your rules still work on the other computers."
        };
        let mut bottom = row.bottom() + 6.0;
        bottom += para(ui, pos2(content.left(), bottom), content.width(), text, regular(T_META), pal().ink2);
        if input.local_rules && let Some(status) = self.running().and_then(|status| status.input.iter()
            .find(|line| line.starts_with("Local key rules "))).cloned() {
            bottom += 6.0;
            bottom += para(ui, pos2(content.left(), bottom), content.width(), &status, regular(T_META), pal().ink2);
        }
        if cfg!(target_os = "macos") {
            bottom += 6.0;
            bottom += para(ui, pos2(content.left(), bottom), content.width(),
                "Simple key swaps, including Caps Lock, use this Mac’s keyboard settings. Caps Lock can only map to one key, not a chord, layer or app-only rule. Fn and media keys are not available.", regular(T_META), pal().ink2);
        }
        if let Some((_, error)) = self.keys.error.clone().filter(|(area, _)| *area == "local-rules") {
            bottom += 6.0 + error_line(ui, content.left(), bottom + 6.0, content.width(), &error);
        }
        card.end(ui, bottom)
    }

    /// "For: All computers ▾": the computers changes are for.
    /// Returns the button's rectangle.
    fn scope_menu(&mut self, ui: &mut Ui, room: Rect) -> Rect {
        let label = format!("For: {}", self.keys.scope.label());
        // The handoff's: `surface2`, 32 tall, padding 12 and 10, 500 13, a chevron.
        let lw = (text_width(ui, &label, medium(T_BODY)) + 42.0).min(room.width()).min(380.0);
        let r = Rect::from_min_size(pos2(room.right() - lw, room.center().y - 16.0), vec2(lw, 32.0));
        let shown = fit(ui.painter(), &label, &medium(T_BODY), lw - 42.0);
        let resp = small_button(ui, r, Id::unique("keys-scope"), "", Some(&format!("{label}. Choose which computers the changes are for")));
        let p = ui.painter();
        p.text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, &shown, medium(T_BODY), pal().ink);
        let c = pos2(r.right() - 17.0, r.center().y + 0.5);
        p.add(egui::Shape::line(vec![c + vec2(-3.0, -1.5), c + vec2(0.0, 1.5), c + vec2(3.0, -1.5)], Stroke::new(1.6, pal().ink)));
        let peers = self.scope_peers();
        // Development aid (screenshots): the menu opens by itself.
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if std::env::var_os("ONE_MK_APP_KEY_SCOPE_MENU").is_some() {
            let once = Id::unique("debug-scope-menu");
            if !ui.ctx().data(|d| d.get_temp::<bool>(once).unwrap_or(false)) {
                ui.ctx().data_mut(|d| d.insert_temp(once, true));
                egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&resp));
                self.link.input_devices();
            }
        }
        if resp.clicked() {
            // Asked afresh each time: devices come and go.
            self.link.input_devices();
        }
        let devices = self.keys.devices.clone();
        let current = self.keys.scope.clone();
        let mut chosen = None;
        egui::Popup::menu(&resp).width(260.0).align(egui::RectAlign::BOTTOM_END).show(|ui| {
            let mut opts = vec![Scope::All, Scope::Os(os::MACOS), Scope::Os(os::WINDOWS), Scope::Os(os::LINUX)];
            opts.extend(peers.iter().map(|p| Scope::Peer(p.name.clone())));
            let heading = |ui: &mut Ui, text: &str| widgets::menu_heading(ui, 260.0, text);
            // A choice: a check when it is the one in force, a second line
            // in small type if given.
            let mut item = |ui: &mut Ui, s: &Scope, title: &str, sub: Option<&str>| {
                let name = match sub {
                    Some(sub) => format!("For: {title}, {sub}"),
                    None => format!("For: {title}"),
                };
                let row = widgets::MenuItem { title, sub, chosen: Some(*s == current), mark: None, name: &name, enabled: true };
                if widgets::menu_item(ui, 260.0, row).clicked() {
                    chosen = Some(s.clone());
                    ui.close();
                }
            };
            for (k, s) in opts.iter().enumerate() {
                if k == 4 {
                    heading(ui, "One computer");
                }
                item(ui, s, &s.label(), None);
            }
            heading(ui, "One keyboard or mouse");
            let note = |ui: &mut Ui, text: &str| {
                let galley = ui.painter().layout(text.to_owned(), regular(T_META), pal().ink2, 236.0);
                let (r, _) = ui.allocate_exact_size(vec2(260.0, galley.size().y + 10.0), Sense::hover());
                ui.painter().galley(r.left_top() + vec2(12.0, 4.0), galley, pal().ink2);
                a11y_text(ui, Id::unique(("keys-scope-note", text)), r, Role::Label, text, None);
            };
            match &devices {
                None => note(ui, "Looking for keyboards and mice…"),
                Some(Err(e)) => note(ui, &format!("Couldn't list the keyboards and mice: {e}")),
                Some(Ok(d)) if !d.available => note(ui, &tune::devices_unavailable_text(d.note.as_deref())),
                Some(Ok(d)) if !d.devices.iter().any(|x| !x.is_virtual()) => {
                    note(ui, "None seen yet. A keyboard or mouse shows up here once it has been connected while oneMK runs.");
                }
                Some(Ok(d)) => {
                    // Devices made by software (X's own XTEST ones) are no
                    // one's keyboard: `one-mk input-devices` lists them.
                    let rows = d.devices.iter().filter(|x| !x.is_virtual()).count().min(5) as f32;
                    let list = egui::ScrollArea::vertical().id_salt("keys-scope-devices").max_height(42.0 * 5.0).min_scrolled_height(42.0 * rows);
                    list.show(ui, |ui| {
                        for dev in d.devices.iter().filter(|x| !x.is_virtual()) {
                            let s = Scope::Device(dev.fingerprint.clone(), Box::new(Scope::All));
                            item(ui, &s, &dev.name, Some(&dev.about()));
                        }
                    });
                    if let Some(n) = &d.note {
                        note(ui, &tune::devices_untested_text(n));
                    }
                }
            }
        });
        if let Some(s) = chosen
            && s != self.keys.scope
        {
            self.keys.scope = s;
            self.keys.capture = None;
            self.keys.picker = None;
        }
        r
    }

    // ---------- keyboard ----------

    /// The 3D keyboard on the floor, at `keyboard_scale`, and (in a window
    /// wide enough) its small toolbar at the stage's top right.
    fn keyboard_stage(&mut self, ui: &mut Ui, stage: Rect, compact: bool, input: &Input, facts: &Facts) {
        let caps = tune::keyboard(self.keys.keypad, mac());
        // What each key does for these computers (plain keys, base layer).
        let mut target: HashMap<u16, What> = HashMap::new();
        for (s, _) in input.specs(facts) {
            if s.layer.is_none() && !s.from.has_mods() && !keynames::is_button(u32::from(s.from.key)) {
                target.entry(s.from.key).or_insert(s.what);
            }
        }
        let drawn: Vec<crate::gear::Cap> = caps
            .iter()
            .map(|c| {
                let label = tune::cap_label(c.code);
                let alpha = label.chars().count() == 1;
                let fkey = label.len() > 1 && label.starts_with('F') && label[1..].chars().all(|d| d.is_ascii_digit());
                let arrow = matches!(c.code, 103 | 105 | 106 | 108);
                crate::gear::Cap { x: c.x, y: c.y, w: c.w, h: c.h, to: target.get(&c.code).map(tune::cap_target), modifier: !alpha && !fkey && !arrow, label }
            })
            .collect();
        // The case is fitted into a rectangle of the handoff's scale, the
        // same whether or not a key is chosen: the camera goes in on the
        // chosen key (from wherever it is now, so choosing the next one
        // glides across) and back out when it is let go; a drag turns it.
        let area = keyboard_rect(stage);
        self.keys.detail_ok = detail_room(stage);
        let chosen = caps.iter().position(|c| self.keys.selected == Some(c.code) && self.keys.layer.is_none());
        let dt = ui.input(|i| i.stable_dt);
        let mut moving = self.keys.board_orbit.step(dt);
        let turn = self.keys.board_orbit;
        moving |= crate::gear::Keyboard::get(&mut self.keys.board, area, &drawn, crate::gear::View::keyboard(turn.yaw, turn.pitch), if self.keys.board_detail && detail_room(stage) { chosen } else { None }, &mut self.keys.board_glide, dt);
        if moving {
            ui.ctx().request_repaint();
        }
        let Some(board) = self.keys.board.take() else { return };

        let held: Vec<u16> = if self.keys.test_focused { self.keys.tester.as_ref().map_or(vec![], |(t, ..)| t.pressed.down().to_vec()) } else { vec![] };
        if !caps.iter().any(|c| c.code == self.keys.focus_key) {
            self.keys.focus_key = 58;
        }
        // One area takes clicks and drags (the whole stage); which key is
        // under the pointer comes from the keys' drawn tops. Each key is
        // focusable on its own, for the keyboard and for screen readers.
        let resp = ui.interact(stage, Id::unique("board"), widgets::sense(ui, Sense::CLICK | Sense::DRAG));
        if resp.dragged() {
            self.keys.board_orbit.pull(resp.drag_delta(), BOARD_TURN.0, BOARD_TURN.1);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if resp.double_clicked() {
            self.keys.board_orbit.reset();
        }
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| stage.contains(*p) && ui.rect_contains_pointer(stage));
        let under = pointer.and_then(|pos| board.hit(pos));
        let under_code = under.map(|i| caps[i].code);
        if under_code != self.keys.board_hover {
            self.keys.board_hover = under_code;
            ui.ctx().request_repaint();
        }
        if under.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        // Tapping away from the keys lets go of the chosen one, and the view
        // zooms back out.
        if resp.clicked() && under.is_none() && self.keys.selected.is_some() {
            self.keys.selected = None;
            self.keys.capture = None;
            self.keys.picker = None;
        }
        if resp.clicked()
            && let Some(i) = under
        {
            self.select_key(caps[i].code, None);
            // The keyboard carries on from the key clicked.
            self.keys.focus_key = caps[i].code;
            ui.memory_mut(|m| m.request_focus(Id::unique(("cap", caps[i].code))));
        }
        let mut go = None;
        let mut focused = None;
        // Every key is named for screen readers; one at a time takes the
        // keyboard (the arrows move it), as in a grid.
        for (i, c) in caps.iter().enumerate() {
            let id = Id::unique(("cap", c.code));
            let roving = c.code == self.keys.focus_key;
            let kr = ui.interact(board.target_bounds(i).intersect(stage), id, widgets::sense(ui, if roving { Sense::focusable_noninteractive() } else { Sense::hover() }));
            if kr.has_focus() {
                focused = Some(i);
                ui.memory_mut(|m| m.set_focus_lock_filter(id, EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
                let (dx, dy) = ui.input(|k| {
                    let d = |a: Key, b: Key| i8::from(k.key_pressed(b)) - i8::from(k.key_pressed(a));
                    (d(Key::ArrowLeft, Key::ArrowRight), d(Key::ArrowUp, Key::ArrowDown))
                });
                if (dx, dy) != (0, 0)
                    && let Some(n) = tune::neighbour(&caps, i, dx, dy)
                {
                    go = Some(caps[n].code);
                }
                if ui.input(|k| k.key_pressed(Key::Enter) || k.key_pressed(Key::Space)) {
                    self.select_key(c.code, None);
                }
            }
            let does = key_description(c.code, target.get(&c.code));
            let selected = self.keys.selected == Some(c.code) && self.keys.layer.is_none();
            widgets::a11y(&kr, Role::Button, &format!("{does}. Arrows move between keys, Enter zooms in to change it"), Some(selected));
            if key_accessibility_activation(ui, &kr) {
                self.select_key(c.code, None);
                self.keys.focus_key = c.code;
                ui.memory_mut(|memory| memory.request_focus(id));
            }
            if kr.has_focus() && widgets::keyboard_focus(ui.ctx()) {
                egui::Tooltip::for_widget(&kr).show(|ui| { ui.label(&does); });
            } else if under == Some(i) && target.contains_key(&c.code) {
                kr.on_hover_text(&does);
            }
        }
        let held_i: Vec<usize> = caps.iter().enumerate().filter(|(_, c)| held.contains(&c.code)).map(|(i, _)| i).collect();
        let painter = ui.painter_at(stage);
        floor(&painter, stage, keyboard_scale(stage.size()));
        board.paint(&painter, &held_i, under, chosen, &|size, _| regular(size));
        // Zoomed in, the board runs past the stage: it fades into the ground
        // there rather than ending in a hard line.
        edge_fade(&painter, stage, 28.0);
        if let Some(i) = focused.filter(|_| widgets::keyboard_focus(ui.ctx())) {
            painter.rect_stroke(board.bounds(i).expand(3.0), CornerRadius::same(6), Stroke::new(2.0, pal().focus), StrokeKind::Outside);
        }
        self.keys.board = Some(board);
        if let Some(code) = go {
            self.keys.focus_key = code;
            ui.memory_mut(|m| m.request_focus(Id::unique(("cap", code))));
        }
        // Over the board (made after it, so it takes its own clicks).
        if !compact {
            self.keyboard_toolbar(ui, Rect::from_min_size(stage.min, vec2(stage.width(), 28.0)), false);
        }
    }

    /// "Choose a key", "Hide keypad" and "Whole board / Key detail": small
    /// quiet buttons at the stage's top right, with what the key under the
    /// pointer does at the left; in a compact window, a row in the panel.
    /// Returns the row's bottom.
    fn keyboard_toolbar(&mut self, ui: &mut Ui, r: Rect, in_panel: bool) -> f32 {
        let small = |ui: &Ui, label: &str| text_width(ui, label, medium(T_META)) + 24.0;
        let keypad = if self.keys.keypad { "Hide keypad" } else { "Show keypad" };
        let detail = self.keys.selected.filter(|_| self.keys.detail_ok).map(|_| if self.keys.board_detail { "Whole board" } else { "Key detail" });
        let chooser = self.keys.selected.map_or_else(|| "Choose a key".to_owned(), |code| tune::key_label(u32::from(code)));
        let chooser_w = (text_width(ui, &chooser, medium(T_META)) + 44.0).clamp(120.0, 200.0);
        // Left to right in the panel; from the right on the stage.
        let mut widths = vec![chooser_w, small(ui, keypad)];
        if let Some(d) = detail {
            widths.push(small(ui, d));
        }
        let total = widths.iter().sum::<f32>() + 6.0 * (widths.len() - 1) as f32;
        let mut x = if in_panel { r.left() } else { r.right() - total };
        let mut rects = widths.iter().map(|w| {
            let b = Rect::from_min_size(pos2(x, r.top()), vec2(*w, 28.0));
            x += w + 6.0;
            b
        });
        let chooser_r = rects.next().expect("three buttons");
        let keypad_r = rects.next().expect("three buttons");
        let detail_r = rects.next();
        if !in_panel {
            let focused = widgets::keyboard_focus(ui.ctx())
                && ui.memory(|memory| memory.has_focus(Id::unique(("cap", self.keys.focus_key))));
            let code = if focused { Some(self.keys.focus_key) } else { self.keys.board_hover.or(self.keys.selected.filter(|_| self.keys.layer.is_none())) };
            let intro = code.map(|code| {
                let facts = self.facts();
                let specs = self.prefs.input.specs(&facts);
                let action = specs.iter().find(|(spec, _)| spec.layer.is_none() && !spec.from.has_mods() && spec.from.key == code).map(|(spec, _)| &spec.what);
                key_description(code, action)
            }).unwrap_or_else(|| "Click a key to zoom · drag to turn".to_owned());
            let iw = chooser_r.left() - r.left() - 16.0;
            let p = ui.painter();
            p.text(pos2(r.left(), r.center().y), Align2::LEFT_CENTER, fit(p, &intro, &regular(T_META), iw), regular(T_META), pal().ink3);
            let intro_rect = Rect::from_min_size(r.min, vec2(iw.max(0.0), r.height()));
            let guidance = "Click a key to zoom in and change it. Whole board zooms back out. Drag to turn the keyboard; double-click to reset its angle.";
            ui.interact(intro_rect, Id::unique("keys-intro-help"), Sense::hover()).on_hover_text(guidance);
            a11y_text(ui, Id::unique("keys-intro"), intro_rect, Role::Label, &intro, None);
            ui.ctx().accesskit_node_builder(Id::unique("keys-intro"), |node| node.set_description(guidance));
        }
        self.keyboard_navigator(ui, chooser_r, &chooser);
        if small_button(ui, keypad_r, Id::unique("keys-more"), keypad, Some(if self.keys.keypad { "Hide the keypad" } else { "Show the keypad too" })).clicked() {
            self.keys.keypad = !self.keys.keypad;
        }
        if let (Some(rect), Some(label)) = (detail_r, detail) {
            let response = small_button(ui, rect, Id::unique("keyboard-detail"), label, None);
            let description = if self.keys.board_detail { "Zoom out to the whole keyboard" } else { "Zoom in on the selected key" };
            ui.ctx().accesskit_node_builder(response.id, |node| node.set_description(description));
            if response.on_hover_text(description).clicked() {
                self.keys.board_detail = !self.keys.board_detail;
            }
        }
        r.top() + 28.0
    }

    /// "Choose a key ▾": every key by name, searchable.
    fn keyboard_navigator(&mut self, ui: &mut Ui, rect: Rect, name: &str) {
        let response = small_button(ui, rect, Id::unique("keyboard-navigator"), "", Some("Choose a key to change"));
        let p = ui.painter();
        p.text(pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, fit(p, name, &medium(T_META), rect.width() - 38.0), medium(T_META), pal().ink);
        let arrow = pos2(rect.right() - 14.0, rect.center().y);
        p.add(egui::Shape::line(vec![arrow + vec2(-4.0, -2.0), arrow + vec2(0.0, 2.0), arrow + vec2(4.0, -2.0)], Stroke::new(1.5, pal().ink2)));
        if response.clicked() { self.keys.navigator_query.clear(); }
        let mut selected = None;
        let facts = self.facts();
        let specs = self.prefs.input.specs(&facts);
        widgets::debug_open_menu(ui, &response, "navigator");
        egui::Popup::menu(&response).width(320.0).show(|ui| {
            // The search: the window's own field (38 is a page's; 32 here).
            let (field, _) = ui.allocate_exact_size(vec2(320.0, 32.0), Sense::hover());
            let search = widgets::field(ui, field, Id::unique("keyboard-navigator-search"), &mut self.keys.navigator_query, "Search keys", "Search keys", true, false);
            ui.add_space(4.0);
            if response.clicked() { search.request_focus(); }
            let enter = search.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            let query = self.keys.navigator_query.trim().to_lowercase();
            let mut matches = 0;
            let mut first_match = None;
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for cap in tune::keyboard(self.keys.keypad, mac()) {
                    let name = tune::key_label(u32::from(cap.code));
                    if !key_matches_query(cap.code, &query) { continue; }
                    matches += 1;
                    first_match.get_or_insert(cap.code);
                    let changed = specs.iter().any(|(spec, _)| spec.layer.is_none() && !spec.from.has_mods() && spec.from.key == cap.code);
                    let label = if changed { format!("{name} · Changed") } else { name };
                    if widgets::menu_item(ui, 308.0, widgets::MenuItem::choice(&label, &label, self.keys.selected == Some(cap.code))).clicked() {
                        selected = Some(cap.code);
                        ui.close();
                    }
                }
            });
            if enter && let Some(code) = first_match {
                selected = Some(code);
                ui.close();
            }
            if matches == 0 {
                let hidden_keypad_match = !self.keys.keypad && tune::keyboard(true, mac()).iter().any(|cap| {
                    key_matches_query(cap.code, &query)
                });
                let text = if hidden_keypad_match { "No matching keys. Show the keypad to find this key." } else { "No matching keys" };
                let g = ui.painter().layout(text.to_owned(), regular(T_BODY), pal().ink2, 300.0);
                let (r, _) = ui.allocate_exact_size(vec2(320.0, g.size().y + 16.0), Sense::hover());
                ui.painter().galley(r.left_top() + vec2(10.0, 8.0), g, pal().ink2);
                a11y_text(ui, Id::unique("keyboard-navigator-none"), r, Role::Label, text, None);
            }
        });
        if let Some(code) = selected { self.select_key(code, None); }
    }

    /// The quick setups' labels as the chips show them.
    fn preset_labels(presets: &[tune::Preset]) -> Vec<String> {
        presets.iter().map(|preset| preset.label.replace(" (for Mac keyboards)", "").replace("Arrow keys on Right ", "Arrows on ")).collect()
    }

    /// A quick setup turned on or off: each rule is checked as `one-mk
    /// keymap add` would, in turn, before anything is changed.
    fn toggle_preset(&mut self, p: &tune::Preset, on: bool, input: &Input) {
        let mut sim = input.clone();
        let mut bad = None;
        if !on {
            for r in &p.rules {
                let r = KeyRule { scope: self.keys.scope.clone(), ..r.clone() };
                if let Err(e) = tune::check_add(&sim, &r) {
                    bad = Some(e);
                    break;
                }
                sim.keys.push(r);
            }
        }
        match bad {
            Some(e) => self.keys.error = Some(("keyboard", format!("“{}” can't be used: {e}", p.label))),
            None => self.tune_cmds(&format!("“{}”", p.label), "keyboard", tune::preset_commands(p, &self.keys.scope, !on), None),
        }
    }

    /// Under the stage: "Quick setups" and a chip for each, centred.
    fn quick_setups_row(&mut self, ui: &mut Ui, r: Rect, input: &Input) {
        let presets = tune::presets(mac());
        let labels = Self::preset_labels(&presets);
        let head = "Quick setups";
        let head_w = text_width(ui, head, regular(T_META)) + 12.0;
        let mut widths: Vec<f32> = labels.iter().map(|l| chip_width(ui, l, false)).collect();
        let chips_w = |widths: &[f32]| widths.iter().sum::<f32>() + 8.0 * (widths.len().saturating_sub(1)) as f32;
        // The heading goes first when room runs short, then the chips narrow.
        let show_head = head_w + chips_w(&widths) <= r.width();
        if chips_w(&widths) > r.width() {
            let k = (r.width() - 8.0 * (widths.len() - 1) as f32) / widths.iter().sum::<f32>();
            widths.iter_mut().for_each(|w| *w *= k);
        }
        let total = chips_w(&widths) + if show_head { head_w } else { 0.0 };
        let mut x = r.center().x - total / 2.0;
        if show_head {
            ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, head, regular(T_META), pal().ink2);
            a11y_text(ui, Id::unique("quick-setups-heading"), Rect::from_min_size(pos2(x, r.top()), vec2(head_w, r.height())), Role::Heading, head, None);
            x += head_w;
        }
        let busy = self.tuning_busy();
        for ((p, label), w) in presets.iter().zip(&labels).zip(widths) {
            let on = tune::preset_on(input, p, &self.keys.scope);
            let rect = Rect::from_min_size(pos2(x, r.center().y - 15.0), vec2(w, 30.0));
            if chip(ui, rect, Id::unique(("preset", p.id)), label, on, !busy, false).on_hover_text(&p.label).clicked() {
                self.toggle_preset(p, on, input);
            }
            x += w + 8.0;
        }
    }

    /// The keyboard's panel: the chosen key (or the toolbar and the list in
    /// a compact window), the changed keys, and "Test it" at the bottom.
    /// Returns how tall it is.
    fn keyboard_panel(&mut self, ui: &mut Ui, at: Pos2, w: f32, viewport: f32, compact: bool, now: f64) -> f32 {
        let input = self.prefs.input.clone();
        let facts = self.facts();
        let mut y = at.y;
        if compact {
            y = self.keyboard_toolbar(ui, Rect::from_min_size(pos2(at.x, y), vec2(w, 28.0)), true) + PANEL_GAP;
        }
        let header_top = y;
        let selected = self.keys.selected.is_some();
        if selected {
            y = self.key_editor(ui, pos2(at.x, y), w, &input, &facts);
        }
        if std::mem::take(&mut self.keys.reveal) {
            ui.scroll_to_rect(Rect::from_min_size(pos2(at.x, header_top - 6.0), vec2(w, 60.0)), None);
        }
        y = self.rules_card(ui, pos2(at.x, y), w, &input, selected);
        if compact {
            y = self.quick_setups_panel(ui, pos2(at.x, y), w, &input);
        }
        // "Test it" at the bottom when there is room, else after the rest.
        let th = self.keys.panel_heights.get("test").copied().unwrap_or(64.0);
        let ty = (y + PANEL_GAP).max(at.y + viewport - th);
        // Pinned to the foot, "Test it" is the panel's footer: a hairline
        // across the panel sets it apart from the list above.
        if ty > y + PANEL_GAP + 24.0 {
            widgets::hairline(ui.painter(), egui::Rangef::new(at.x, at.x + w), ty - PANEL_GAP);
        }
        let measured = self.test_card(ui, pos2(at.x, ty), w, now);
        if (th - measured).abs() > 0.5 {
            self.keys.panel_heights.insert("test", measured);
            ui.ctx().request_repaint();
        }
        ty + measured - at.y
    }

    /// In a compact window the quick setups are a row in the panel.
    fn quick_setups_panel(&mut self, ui: &mut Ui, at: Pos2, w: f32, input: &Input) -> f32 {
        let card = card_begin(ui, at, w, "Quick setups");
        let c = card.inner;
        let presets = tune::presets(mac());
        let labels = Self::preset_labels(&presets);
        let widths: Vec<f32> = labels.iter().map(|l| chip_width(ui, l, false)).collect();
        let (rects, bottom) = quick_setup_rects(c.width(), 0.0, &widths);
        let busy = self.tuning_busy();
        for ((p, label), rect) in presets.iter().zip(&labels).zip(rects) {
            let on = tune::preset_on(input, p, &self.keys.scope);
            // No heading beside them here: the rows start at the top.
            let rect = rect.translate(vec2(c.left(), c.top() - 38.0));
            if chip(ui, rect, Id::unique(("preset", p.id)), label, on, !busy, true).on_hover_text(&p.label).clicked() {
                self.toggle_preset(p, on, input);
            }
        }
        c.top() - 38.0 + bottom
    }

    /// The changed keys (handoff 5.5): rows 34 tall, the chosen key's rule
    /// on `accent_soft`, a tag at the right ("Macs"), and ✎ / ✕ under the
    /// pointer; with what `one-mk keymap check` found above them. With no
    /// key chosen it heads the panel. Returns its bottom.
    fn rules_card(&mut self, ui: &mut Ui, at: Pos2, w: f32, input: &Input, below: bool) -> f32 {
        let n = input.keys.len();
        let title = if n == 0 { "Changed keys".to_owned() } else { format!("Changed keys ({n})") };
        let mut y = if below {
            card_begin(ui, at, w, &title).inner.top()
        } else {
            ui.painter().text(pos2(at.x, at.y + 11.0), Align2::LEFT_CENTER, &title, semibold(T_TITLE), pal().ink);
            a11y_text(ui, Id::unique(("card", "Changed keys")), Rect::from_min_size(at, vec2(w, 22.0)), Role::Heading, &title, None);
            at.y + 22.0 + 12.0
        };
        let (left, width) = (at.x, w);
        if self.prefs.unreadable {
            y += error_line(ui, left, y, width, "Preferences couldn't be read. Changes are paused to keep your saved choices safe. Try restarting oneMK.") + 8.0;
        }
        if let Some((_, e)) = self.keys.error.clone().filter(|(a, _)| *a == "list") {
            y += error_line(ui, left, y, width, &e) + 8.0;
        }
        if !below && let Some((_, e)) = self.keys.error.clone().filter(|(a, _)| *a == "keyboard") {
            y += error_line(ui, left, y, width, &e) + 8.0;
        }
        match &self.keys.problems {
            Some(Ok(list)) => {
                for prob in list {
                    y += error_line(ui, left, y, width, &tune::plain_problem(prob)) + 6.0;
                }
            }
            Some(Err(e)) => y += error_line(ui, left, y, width, &format!("The rules couldn't be checked: {e}")) + 6.0,
            None => {}
        }
        if n == 0 {
            // An empty state that looks meant: the keys' icon on a soft tile
            // beside the sentence, in a quiet well.
            let text = "No keys are changed yet. Click a key above, or start with a quick setup.";
            let tw = width - 28.0 - 48.0;
            let th = widgets::para_code_h(ui, tw, text, regular(T_BODY));
            let well = Rect::from_min_size(pos2(left, y), vec2(width, (th + 28.0).max(64.0)));
            widgets::well(ui.painter(), well);
            let tile = Rect::from_min_size(pos2(well.left() + 14.0, well.top() + 14.0), vec2(36.0, 36.0));
            ui.painter().rect(tile, CornerRadius::same(10), pal().accent_soft, Stroke::new(1.0, pal().line), StrokeKind::Inside);
            crate::icons::draw(ui.painter(), crate::icons::Icon::Keys, Rect::from_center_size(tile.center(), vec2(18.0, 18.0)), pal().accent_text);
            para(ui, pos2(tile.right() + 12.0, well.center().y - th / 2.0), tw, text, regular(T_BODY), pal().ink2);
            return well.bottom();
        }
        let busy = self.tuning_busy();
        let chosen = self.keys.selected.and_then(|code| self.own_rule(code)).map(|(i, _)| i);
        let mut act = None;
        for (i, r) in input.keys.iter().enumerate() {
            let row = Rect::from_min_size(pos2(left, y), vec2(width, 34.0));
            let (text, ok) = match tune::sentence(input, r) {
                Ok(s) => (s, true),
                Err(e) => (tune::plain_problem(&e), false),
            };
            let scope_tag = match &r.scope {
                Scope::All => String::new(),
                s => s.label(),
            };
            let applies_here = input.local_rules && cfg!(any(windows, target_os = "macos"))
                && r.scope.includes_local_os(one_mk_proto::os::local());
            let tag = if applies_here {
                if scope_tag.is_empty() { "Also here".into() } else { format!("{scope_tag} · Also here") }
            } else { scope_tag };
            let on = chosen == Some(i);
            let edit = Rect::from_center_size(pos2(row.right() - 46.0, row.center().y), vec2(26.0, 26.0));
            let del = Rect::from_center_size(pos2(row.right() - 17.0, row.center().y), vec2(26.0, 26.0));
            // The row: a click opens the rule, as ✎ does.
            let resp = ui.interact(Rect::from_min_max(row.min, pos2(edit.left() - 4.0, row.bottom())), Id::unique(("rule-text", i)), widgets::sense(ui, Sense::click()));
            let lit = ui.rect_contains_pointer(row) || ui.memory(|m| {
                [Id::unique(("rule-edit", i)), Id::unique(("rule-del", i)), resp.id].iter().any(|id| m.has_focus(*id))
            });
            let hover = tween(ui.ctx(), resp.id.with("hover"), f32::from(lit), 0.1, motion::out_cubic);
            let fill = if on { pal().accent_soft } else { mix(Color32::TRANSPARENT, pal().hover, hover) };
            ui.painter().rect_filled(row, CornerRadius::same(8), fill);
            widgets::focus_ring(ui, &resp, row, 8);
            let tag_w = if tag.is_empty() || lit { 0.0 } else { text_width(ui, &tag, regular(T_CHIP)).min(width * 0.3) + 10.0 };
            let x = if ok {
                row.left() + 10.0
            } else {
                ui.painter().circle_filled(pos2(row.left() + 13.0, row.center().y), 3.0, pal().danger);
                row.left() + 22.0
            };
            let room = if lit { edit.left() - 6.0 - x } else { row.right() - 10.0 - tag_w - x };
            let font = if on { semibold(T_BODY) } else { regular(T_BODY) };
            let shown = fit(ui.painter(), &text, &font, room);
            ui.painter().text(pos2(x, row.center().y), Align2::LEFT_CENTER, &shown, font, if on { pal().accent_text } else { pal().ink });
            if tag_w > 0.0 {
                let t = fit(ui.painter(), &tag, &regular(T_CHIP), tag_w - 10.0);
                ui.painter().text(pos2(row.right() - 10.0, row.center().y), Align2::RIGHT_CENTER, t, regular(T_CHIP), pal().ink3);
            }
            let full = if tag.is_empty() { text.clone() } else { format!("{text} ({tag})") };
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let editable = r.from.parse::<keynames::Trigger>().is_ok_and(|t| !t.has_mods());
            widgets::a11y(&resp, Role::ListItem, &full, Some(on));
            let resp = if !ok {
                resp.on_hover_text(format!("{}\n{full}", tune::as_written(r)))
            } else if shown != text {
                resp.on_hover_text(&full)
            } else {
                resp
            };
            if resp.clicked() && editable && !busy {
                act = Some((i, true));
            }
            // ✎ and ✕ show under the pointer (and while one has the keyboard).
            if lit {
                if widgets::icon_button(ui, edit, Id::unique(("rule-edit", i)), &format!("Change: {text}"), editable && !busy, draw_pencil).clicked() {
                    act = Some((i, true));
                }
                if widgets::icon_button(ui, del, Id::unique(("rule-del", i)), &format!("Remove: {text}"), !busy, draw_cross).clicked() {
                    act = Some((i, false));
                }
            } else {
                // Still reachable with Tab: the buttons stay in the order,
                // drawn only once they have the keyboard or the pointer.
                for (rect, id, name) in [(edit, Id::unique(("rule-edit", i)), format!("Change: {text}")), (del, Id::unique(("rule-del", i)), format!("Remove: {text}"))] {
                    let r = ui.interact(rect, id, widgets::sense(ui, Sense::click()));
                    widgets::a11y(&r, Role::Button, &name, None);
                }
            }
            y += 34.0 + 2.0;
        }
        match act {
            Some((i, true)) => {
                let r = input.keys[i].clone();
                if let Ok(t) = r.from.parse::<keynames::Trigger>() {
                    self.keys.scope = r.scope.clone();
                    self.select_key(t.key, r.layer.clone());
                    self.keys.app = r.app.clone();
                }
            }
            Some((i, false)) => {
                let r = &input.keys[i];
                let what = format!("the rule for {}", r.from.parse::<keynames::Trigger>().map_or(r.from.clone(), |t| tune::trigger_text(&t)));
                self.tune_cmds(&what, "list", vec![tune::remove_args(r)], None);
            }
            None => {}
        }
        y - 2.0
    }

    /// The chosen key (handoff 5.5): its chip, name and what it does now;
    /// When tapped, When held, Held means after, Works in; "Press new key"
    /// and "Choose from list", with "No output" and "Reset" under "…".
    /// Returns its bottom.
    fn key_editor(&mut self, ui: &mut Ui, at: Pos2, w: f32, input: &Input, facts: &Facts) -> f32 {
        let code = self.keys.selected.unwrap_or(58);
        let name = tune::key_label(u32::from(code));
        let title = match &self.keys.layer {
            Some(l) => match input.layer_key(l) {
                Some(k) => format!("{} + {name}", tune::key_label(u32::from(k))),
                None => format!("{name} in “{l}”"),
            },
            None => name.clone(),
        };
        let own = self.own_rule(code).map(|(_, r)| r);
        let effective = input
            .specs(facts)
            .into_iter()
            .find(|(s, _)| s.from == keynames::Trigger::key(code) && s.layer.as_deref() == self.keys.layer.as_deref().map(str::to_ascii_lowercase).as_deref());
        let shown = effective.as_ref().map(|(_, at)| *at);
        let now_text = match (&own, shown) {
            (Some(r), _) => tune::sentence(input, r).map(|s| s.split_once(" → ").map_or(s.clone(), |(_, t)| t.to_owned())).unwrap_or_else(|e| tune::plain_problem(&e)),
            (None, Some(Ok(i))) => {
                let r = &input.keys[i];
                let s = tune::sentence(input, r).unwrap_or_default();
                format!("{} ({})", s.split_once(" → ").map_or(s.as_str(), |(_, t)| t), r.scope.label().to_lowercase())
            }
            _ => format!("Types {name}, as it is."),
        };
        let (tapped, held) = tap_hold_text(effective.as_ref().map(|(s, _)| &s.what), &name);

        // The key's chip, its name and what it does now; ✕ lets it go.
        let p = ui.painter().clone();
        let cap = tune::cap_label(code);
        let galley = p.layout_no_wrap(cap.clone(), mono_medium(T_META), pal().on_accent);
        let chip_r = Rect::from_min_size(pos2(at.x, at.y + 4.0), vec2((galley.size().x + 18.0).max(32.0), 32.0));
        p.rect_filled(chip_r, CornerRadius::same(8), pal().accent);
        p.galley(chip_r.center() - galley.size() / 2.0, galley, pal().on_accent);
        let close = Rect::from_center_size(pos2(at.x + w - 13.0, at.y + 13.0), vec2(26.0, 26.0));
        let more = Rect::from_center_size(pos2(close.center().x - 30.0, close.center().y), vec2(26.0, 26.0));
        let tx = chip_r.right() + 12.0;
        let room = more.left() - 6.0 - tx;
        p.text(pos2(tx, at.y + 11.0), Align2::LEFT_CENTER, fit(&p, &title, &semibold(T_TITLE), room), semibold(T_TITLE), pal().ink);
        let sub = fit(&p, &now_text, &regular(T_META), room);
        p.text(pos2(tx, at.y + 31.0), Align2::LEFT_CENTER, &sub, regular(T_META), pal().ink2);
        let head = Rect::from_min_max(at, pos2(close.left(), at.y + 40.0));
        a11y_text(ui, Id::unique("key-editor-title"), head, Role::Heading, &format!("{title}: {now_text}"), None);
        if sub != now_text {
            ui.interact(head, Id::unique("key-editor-now"), Sense::hover()).on_hover_text(&now_text);
        }
        let esc = !self.keys_grab(ui.ctx()) && self.keys.picker.is_none() && ui.input(|i| i.key_pressed(Key::Escape)) && !ui.ctx().text_edit_focused();
        if widgets::icon_button(ui, close, Id::unique("key-close"), &format!("Done with {title}"), true, draw_cross).clicked() || esc {
            self.keys.selected = None;
            self.keys.capture = None;
            self.keys.picker = None;
            ui.memory_mut(|m| m.request_focus(Id::unique(("cap", code))));
        }
        let busy = self.tuning_busy();
        // "…": No output and Reset.
        let own_rule = own.clone();
        let menu = widgets::icon_button(ui, more, Id::unique("key-act-more"), &format!("More for {name}: no output, reset"), !busy, widgets::draw_dots);
        let mut pick = None;
        widgets::debug_open_menu(ui, &menu, "key-more");
        egui::Popup::menu(&menu).width(220.0).align(egui::RectAlign::BOTTOM_END).show(|ui| {
            let mut item = |ui: &mut Ui, k: usize, text: &str, full: String| {
                if widgets::menu_item(ui, 220.0, widgets::MenuItem::command(text, &full)).clicked() && !busy {
                    pick = Some(k);
                    ui.close();
                }
            };
            item(ui, 2, "No output", format!("Make {name} do nothing"));
            if own_rule.is_some() {
                item(ui, 3, "Reset", format!("Make {name} type itself again"));
            }
        });
        match pick {
            Some(2) => self.choose(Want::To, "none".into()),
            Some(_) => {
                if let Some(r) = &own_rule {
                    self.tune_cmds(&name, "keyboard", vec![tune::remove_args(r)], None);
                }
            }
            None => {}
        }
        let mut y = at.y + 40.0 + PANEL_GAP - 10.0;

        // The rows, 46 tall, a hairline between.
        let row = |y: f32| Rect::from_min_size(pos2(at.x, y), vec2(w, 46.0));
        let label = |ui: &Ui, r: Rect, text: &str| {
            ui.painter().text(r.left_center(), Align2::LEFT_CENTER, text, regular(T_BODY), pal().ink);
        };
        let r = row(y);
        label(ui, r, "When tapped");
        kbd_fit(ui.painter(), r.right_center(), Align2::RIGHT_CENTER, &tapped, w - 110.0);
        widgets::hairline(ui.painter(), r.x_range(), r.bottom());
        y += 46.0;

        // When held: a click opens what else it can give when held.
        let r = row(y);
        let open = self.keys.more;
        let resp = ui.interact(r, Id::unique("key-more"), widgets::sense(ui, Sense::click()));
        let hover = tween(ui.ctx(), resp.id.with("hover"), f32::from(resp.hovered()), 0.1, motion::out_cubic);
        ui.painter().rect_filled(r.expand2(vec2(6.0, -4.0)), CornerRadius::same(8), mix(Color32::TRANSPARENT, pal().hover, hover));
        widgets::focus_ring(ui, &resp, r.expand2(vec2(6.0, -4.0)), 8);
        label(ui, r, "When held");
        let m = pos2(r.left() + text_width(ui, "When held", regular(T_BODY)) + 12.0, r.center().y);
        if open {
            ui.painter().add(egui::Shape::line(vec![m + vec2(-4.0, -2.0), m + vec2(0.0, 2.0), m + vec2(4.0, -2.0)], Stroke::new(1.5, pal().ink3)));
        } else {
            crate::icons::draw(ui.painter(), crate::icons::Icon::Chevron, Rect::from_center_size(m, vec2(14.0, 14.0)), pal().ink3);
        }
        kbd_fit(ui.painter(), r.right_center(), Align2::RIGHT_CENTER, &held, w - 110.0);
        let more_label = "More: a different key when held";
        resp.widget_info(move || egui::WidgetInfo::selected(Role::Button, true, open, more_label));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            self.keys.more = !open;
        }
        y += 46.0;
        if open {
            let hold = own.as_ref().and_then(|r| r.hold.clone());
            let hold_text = hold.as_ref().and_then(|h| h.parse().ok()).map_or("nothing else yet: it types the same".to_owned(), |o| tune::out_text(&o));
            y += para(ui, pos2(at.x, y), w, &format!("Tapped, it types what is chosen above. Held, it gives: {hold_text}."), regular(T_META), pal().ink2) + 8.0;
            if let Some((Want::Hold, _)) = self.keys.capture {
                y = self.capture_box(ui, at.x, y, w, &name, Want::Hold);
            } else if let Some((Want::Hold, query)) = self.keys.picker.clone() {
                y = self.picker(ui, at.x, y, w, Want::Hold, query, 6);
            } else {
                let mut labels = vec!["Press the key for holding", "Choose from a list"];
                if hold.is_some() {
                    labels.push("Not when held");
                }
                let (rects, bottom) = flow(ui, at.x, y, w, &labels);
                for (k, r) in rects.iter().enumerate() {
                    let names = [format!("Press the key {name} gives when held"), format!("Choose what {name} gives when held"), format!("Make {name} the same when held")];
                    if button(ui, *r, Id::unique(("key-hold", k)), labels[k], Look::Quiet, !busy, Some(&names[k])).clicked() {
                        match k {
                            0 => self.keys.capture = Some((Want::Hold, Capture::default())),
                            1 => self.keys.picker = Some((Want::Hold, String::new())),
                            _ => {
                                if let Some(r) = own.clone() {
                                    let tap = r.tap.clone().unwrap_or_else(|| r.from.clone());
                                    self.save_rule(KeyRule { to: Some(tune::To::One(tap)), tap: None, hold: None, timeout_ms: None, ..r });
                                }
                            }
                        }
                    }
                }
                y = bottom + 8.0;
            }
        }
        widgets::hairline(ui.painter(), egui::Rangef::new(at.x, at.x + w), y);

        // Held means after: editable once the key has a hold.
        let r = row(y);
        label(ui, r, "Held means after");
        let cur = own.as_ref().and_then(|r| r.timeout_ms).unwrap_or(input.tap_ms.unwrap_or(200)).to_string();
        match own.clone().filter(|r| r.hold.is_some()) {
            Some(rule) => {
                let ms_w = text_width(ui, "ms", mono(T_META)) + 6.0;
                ui.painter().text(r.right_center(), Align2::RIGHT_CENTER, "ms", mono(T_META), pal().ink2);
                let fr = Rect::from_min_size(pos2(r.right() - ms_w - 60.0, r.center().y - 14.0), vec2(60.0, 28.0));
                if let Some(text) = self.edit_field(ui, fr, "key-timeout", &cur, "200", "Held after this many milliseconds", !busy, true) {
                    match text.parse::<u32>() {
                        Ok(ms) if (one_mk_proto::remap::MIN_TAP_MS..=one_mk_proto::remap::MAX_TAP_MS).contains(&ms) => {
                            self.save_rule(KeyRule { timeout_ms: Some(ms), ..rule });
                        }
                        _ => self.keys.error = Some(("keyboard", format!("Type a time from {} to {} ms.", one_mk_proto::remap::MIN_TAP_MS, one_mk_proto::remap::MAX_TAP_MS))),
                    }
                }
            }
            None => {
                widgets::mono_text(ui.painter(), r.right_center(), Align2::RIGHT_CENTER, &format!("{cur} ms"), T_META, pal().ink2, 120.0);
            }
        }
        widgets::hairline(ui.painter(), r.x_range(), r.bottom());
        y += 46.0;
        y = self.app_field(ui, pos2(at.x, y), w, &name);

        // The actions, or the key being pressed, or the list.
        y += PANEL_GAP - 6.0;
        match (self.keys.capture, self.keys.picker.clone()) {
            (Some((Want::To, _)), _) => y = self.capture_box(ui, at.x, y, w, &name, Want::To),
            (_, Some((Want::To, query))) => y = self.picker(ui, at.x, y, w, Want::To, query, 7),
            _ => {
                let bw = (w - 8.0) / 2.0;
                let press = Rect::from_min_size(pos2(at.x, y), vec2(bw, 36.0));
                let list = Rect::from_min_size(pos2(press.right() + 8.0, y), vec2(bw, 36.0));
                if widgets::content_button(ui, press, Id::unique(("key-act", 0)), "Press new key", Look::Primary, !busy, Some(&format!("Press the key {name} should become"))).clicked() {
                    self.keys.capture = Some((Want::To, Capture::default()));
                }
                if widgets::content_button(ui, list, Id::unique(("key-act", 1)), "Choose from list", Look::Quiet, !busy, Some(&format!("Choose what {name} becomes from a list"))).clicked() {
                    self.keys.picker = Some((Want::To, String::new()));
                }
                y += 36.0;
            }
        }
        if let Some((_, e)) = self.keys.error.clone().filter(|(a, _)| *a == "keyboard") {
            y += 8.0;
            y += error_line(ui, at.x, y, w, &e);
        }
        let scope_note = match &self.keys.scope {
            Scope::All => "Changes here are for every computer.".to_owned(),
            Scope::Os(o) => format!("Changes here are only for {}.", tune::os_plural(*o)),
            Scope::Peer(p) => format!("Changes here are only for {p}."),
            Scope::Device(fp, _) => format!("Changes here are only for keys typed on {}.", tune::device_name(fp)),
        };
        let scope_note = match &self.keys.app {
            Some(a) => format!("{}, {}.", scope_note.trim_end_matches('.'), tune::app_text(a)),
            None => scope_note,
        };
        y += 10.0;
        y + para(ui, pos2(at.x, y), w, &scope_note, regular(T_META), pal().ink3)
    }

    /// "Works in": the rule being changed applies only while this app has
    /// the focus on the computer typed on ("Only in app"). Changing it moves
    /// the rule there; with no rule yet, the next change is made for that
    /// app. Returns its bottom.
    fn app_field(&mut self, ui: &mut Ui, at: Pos2, w: f32, name: &str) -> f32 {
        let own = self.keys.selected.and_then(|code| self.own_rule(code)).map(|(_, r)| r);
        let own = own.as_ref();
        let busy = self.tuning_busy();
        let row = Rect::from_min_size(at, vec2(w, 46.0));
        ui.painter().text(row.left_center(), Align2::LEFT_CENTER, "Works in", regular(T_BODY), pal().ink);
        let fw = (w - text_width(ui, "Works in", regular(T_BODY)) - 16.0).min(170.0);
        let fr = Rect::from_min_size(pos2(row.right() - fw, row.center().y - 14.0), vec2(fw, 28.0));
        let cur = self.keys.app.clone().unwrap_or_default();
        let help = "Only in app: the program's name on the other computer, like chrome.exe on Windows, com.apple.Terminal on a Mac or firefox on Linux. Empty for every app.";
        if let Some(text) = self.edit_field(ui, fr, "key-app", &cur, "Every app", help, !busy, true) {
            let new = (!text.is_empty()).then_some(text);
            if let Some(Err(e)) = new.as_ref().map(|a| a.parse::<one_mk_proto::appmatch::AppPattern>()) {
                self.keys.error = Some(("keyboard", format!("Only in app: {}", e.trim_start_matches("app: "))));
                return row.bottom();
            }
            if let Some(r) = own.filter(|r| !tune::same_app(&r.app, &new)) {
                let moved = KeyRule { app: new.clone(), ..r.clone() };
                if let Err(e) = tune::check_add(&self.prefs.input, &moved) {
                    self.keys.error = Some(("keyboard", e));
                    return row.bottom();
                }
                self.tune_cmds(name, "keyboard", vec![tune::remove_args(r), tune::add_args(&moved)], None);
            }
            self.keys.app = new;
        }
        let mut y = row.bottom();
        // Say so where the control is when the computers it is for may not
        // be able to tell which app has the focus.
        let covered: Vec<u8> = self.scope_peers().iter().filter(|p| scope_covers(&self.keys.scope, p)).map(|p| p.os).collect();
        if let Some(note) = crate::desktop::app_rule_note(self.running().map_or(&[][..], |s| &s.tuning[..]), &covered) {
            y += para(ui, pos2(at.x, y), w, &note, regular(T_META), pal().ink3) + 4.0;
        }
        y
    }

    /// "Press the key you want…", while the window waits for it.
    fn capture_box(&mut self, ui: &mut Ui, left: f32, y: f32, w: f32, name: &str, want: Want) -> f32 {
        let ask = match want {
            Want::To => format!("Press the key {name} should become…"),
            Want::Hold => format!("Press the key {name} gives when held…"),
            Want::Button => "Press the key or shortcut it should give…".to_owned(),
        };
        let note = "Esc cancels. Shortcuts work too. The keypad, Fn and a few others can't be pressed here: choose them from the list.";
        let bw = button_width(ui, "Cancel") - 6.0;
        let tw = w - bw - 38.0;
        // Measure first: the box is as tall as what it says.
        let lay = |text: &str, font: egui::FontId| {
            let mut job = egui::text::LayoutJob::simple(text.to_owned(), font.clone(), pal().ink, tw);
            job.sections[0].format.line_height = Some((font.size * LEADING).round());
            ui.painter().layout_job(job).size().y
        };
        let (ah, nh) = (lay(&ask, semibold(T_BODY)), lay(note, regular(T_META)));
        let r = Rect::from_min_size(pos2(left, y), vec2(w, (12.0 + ah + 4.0 + nh + 12.0).max(52.0)));
        let id = Id::unique(CAPTURE_ID);
        let resp = ui.interact(r, id, Sense::focusable_noninteractive());
        if !ui.clip_rect().contains_rect(r.expand(4.0)) {
            ui.scroll_to_rect(r.expand(6.0), None);
        }
        ui.painter().rect(r, CornerRadius::same(9), pal().bg, Stroke::new(1.5, pal().accent), StrokeKind::Inside);
        para(ui, pos2(left + 14.0, y + 12.0), tw, &ask, semibold(T_BODY), pal().ink);
        para(ui, pos2(left + 14.0, y + 16.0 + ah), tw, note, regular(T_META), pal().ink2);
        let full = format!("{ask} Esc cancels. Note: {}.", tune::NOT_REPORTED);
        widgets::a11y(&resp, Role::Label, &full, None);
        let resp = ui.interact(r.shrink2(vec2(bw + 20.0, 0.0)).translate(vec2(-(bw + 20.0) / 2.0, 0.0)), id.with("tip"), Sense::hover());
        resp.on_hover_text(format!("Note: {}.", tune::NOT_REPORTED));
        let br = Rect::from_min_size(pos2(r.right() - 10.0 - bw, y + 10.0), vec2(bw, 28.0));
        if button(ui, br, Id::unique("capture-cancel"), "Cancel", Look::Quiet, true, Some("Cancel: don't change anything")).clicked() {
            self.keys.capture = None;
        }
        r.bottom() + 8.0
    }

    /// The searchable list of what a key or button can become.
    #[allow(clippy::too_many_arguments)]
    fn picker(&mut self, ui: &mut Ui, left: f32, y: f32, w: f32, want: Want, query: String, rows: usize) -> f32 {
        let buttons = want == Want::Button;
        if self.keys.choices.as_ref().is_none_or(|(b, _)| *b != buttons) {
            self.keys.choices = Some((buttons, tune::choices(buttons)));
        }
        let bw = button_width(ui, "Cancel") - 6.0;
        let fr = Rect::from_min_size(pos2(left, y), vec2(w - bw - 8.0, 30.0));
        let fid = Id::unique(("field", "key-search"));
        let first = !ui.memory(|m| m.has_focus(fid)) && query.is_empty() && !ui.ctx().data(|d| d.get_temp::<bool>(fid.with("opened")).unwrap_or(false));
        if first {
            ui.memory_mut(|m| m.request_focus(fid));
            ui.ctx().data_mut(|d| d.insert_temp(fid.with("opened"), true));
        }
        let mut q = query;
        let resp = widgets::field(ui, fr, fid, &mut q, "Search: esc, volume, next computer…", "Search for a key or an action", true, false);
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if button(ui, Rect::from_min_size(pos2(fr.right() + 8.0, y), vec2(bw, 30.0)), Id::unique("pick-cancel"), "Cancel", Look::Quiet, true, Some("Close the list")).clicked()
            || (resp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)))
        {
            self.keys.picker = None;
            ui.ctx().data_mut(|d| d.remove::<bool>(fid.with("opened")));
            return y + 38.0;
        }
        let all = &self.keys.choices.as_ref().expect("filled above").1;
        let hits: Vec<tune::Choice> = tune::search(all, &q).into_iter().cloned().collect();
        self.keys.picker = Some((want, q));
        let mut picked = enter.then(|| hits.first().map(|c| c.value.clone())).flatten();
        let rows = hits.len().clamp(3, rows);
        let list = Rect::from_min_size(pos2(left, y + 38.0), vec2(w, 30.0 * rows as f32 + 6.0));
        let controls = Rect::from_min_max(fr.min, list.max).expand(4.0);
        if !ui.clip_rect().contains_rect(controls) {
            ui.scroll_to_rect(controls, None);
        }
        ui.painter().rect(list, CornerRadius::same(8), pal().bg, Stroke::new(1.0, pal().line), StrokeKind::Inside);
        if hits.is_empty() {
            ui.painter().text(list.center(), Align2::CENTER_CENTER, "Nothing is called that. Try another word.", regular(T_BODY), pal().ink2);
        }
        let mut child = ui.new_child(UiBuilder::new().max_rect(list.shrink(3.0)));
        child.set_clip_rect(list.shrink(2.0));
        egui::ScrollArea::vertical().id_salt(("pick-list", buttons)).auto_shrink([false, false]).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (k, c) in hits.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), widgets::sense(ui, Sense::click()));
                let hover = tween(ui.ctx(), resp.id.with("hover"), f32::from(resp.hovered() || resp.has_focus()), 0.1, motion::out_cubic);
                let p = ui.painter();
                p.rect_filled(r, CornerRadius::same(6), mix(Color32::TRANSPARENT, pal().hover, hover));
                widgets::focus_ring(ui, &resp, r.shrink(2.0), 5);
                let vw = text_width(ui, &c.value, mono(T_META)).min(r.width() * 0.4);
                p.text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, fit(p, &c.label, &regular(T_BODY), r.width() - vw - 30.0), regular(T_BODY), pal().ink);
                widgets::mono_text(p, pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, &c.value, T_META, pal().ink2, vw);
                widgets::a11y(&resp, Role::ListItem, &c.label, None);
                if resp.clicked() {
                    picked = Some(c.value.clone());
                }
                let _ = k;
            }
        });
        if let Some(v) = picked {
            ui.ctx().data_mut(|d| d.remove::<bool>(fid.with("opened")));
            self.choose(want, v);
        }
        list.bottom() + 8.0
    }

    /// "Test it" (handoff 5.5): a field (38 tall, `bg`, a `line2` edge) where
    /// keys typed go through the rules as a visit would send them; nothing
    /// is sent and no action runs. Returns how tall it is.
    fn test_card(&mut self, ui: &mut Ui, at: Pos2, w: f32, now: f64) -> f32 {
        let p = ui.painter().clone();
        widgets::section_label(&p, pos2(at.x, at.y + 8.0), Align2::LEFT_CENTER, "Test it", w);
        let has = self.keys.tester.as_ref().is_some_and(|(t, ..)| !t.pressed.chips.is_empty() || t.note.is_some());
        let clear = Rect::from_center_size(pos2(at.x + w - 10.0, at.y + 8.0), vec2(22.0, 22.0));
        if has && widgets::icon_button(ui, clear, Id::unique("test-clear"), "Clear the test box", true, draw_cross).clicked()
            && let Some((t, ..)) = &mut self.keys.tester
        {
            t.clear();
        }
        let intro = match &self.keys.scope {
            Scope::All => "Type in the box to see what every computer would get. Nothing is sent.".to_owned(),
            Scope::Os(o) => format!("Type in the box to see what {} would get. Nothing is sent.", tune::os_plural(*o)),
            Scope::Peer(p) => format!("Type in the box to see what {p} would get. Nothing is sent."),
            Scope::Device(fp, _) => format!("Type in the box to see what typing on {} would send. Nothing is sent.", tune::device_name(fp)),
        };
        let mut y = at.y + 22.0;
        let id = Id::unique(TEST_ID);
        let empty = self.keys.tester.as_ref().is_none_or(|(t, ..)| t.pressed.chips.is_empty());
        // One line until something is typed, then a line for each side.
        let r = Rect::from_min_size(pos2(at.x, y), vec2(w, if empty && self.keys.tester_error.is_none() { 38.0 } else { 78.0 }));
        let resp = ui.interact(r, id, widgets::sense(ui, Sense::click()));
        if resp.clicked() {
            ui.memory_mut(|m| m.request_focus(id));
        }
        let focused = resp.has_focus();
        let _ = self.tester();
        widgets::paint_field(&p, r, resp.hovered(), focused, true);
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        let lw = text_width(ui, "They get", regular(T_META)).max(text_width(ui, "You press", regular(T_META))) + 14.0;
        let mut spoken = String::new();
        if let Some(e) = &self.keys.tester_error {
            para(ui, pos2(r.left() + 12.0, r.top() + 10.0), r.width() - 24.0, &format!("These rules can't be tried: {e}"), regular(T_META), pal().ink);
        } else if empty {
            let hint = if focused { "Type now: try the keys you changed. Tab leaves." } else { "Type here · nothing is sent" };
            p.text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, fit(&p, hint, &regular(T_BODY), r.width() - 24.0), regular(T_BODY), pal().ink3);
        } else if let Some((t, ..)) = &self.keys.tester {
            for (k, (label, log)) in [("You press", &t.pressed), ("They get", &t.sent)].into_iter().enumerate() {
                let ly = r.top() + 21.0 + 36.0 * k as f32;
                p.text(pos2(r.left() + 12.0, ly), Align2::LEFT_CENTER, label, regular(T_META), pal().ink2);
                // Newest last; the oldest drop off the left when there's no room.
                let mut x = r.right() - 10.0;
                let mut shown = vec![];
                for chip in log.chips.iter().rev() {
                    let text = match &chip.note {
                        Some(n) => format!("{} · {n}", chip.text),
                        None => chip.text.clone(),
                    };
                    let cw = text_width(ui, &text, mono(T_META)) + 16.0;
                    if x - cw < r.left() + 12.0 + lw {
                        break;
                    }
                    x -= cw;
                    shown.push((Rect::from_min_size(pos2(x, ly - 11.0), vec2(cw, 22.0)), text));
                    x -= 6.0;
                }
                let dx = shown.last().map_or(0.0, |(rr, _)| rr.left() - (r.left() + 12.0 + lw));
                for (rr, text) in shown.iter().rev() {
                    let rr = rr.translate(vec2(-dx, 0.0));
                    p.rect_filled(rr, CornerRadius::same(6), pal().surface2);
                    widgets::mono_text(&p, rr.center(), Align2::CENTER_CENTER, text, T_META, pal().ink, rr.width());
                }
                spoken += &format!("{label}: {}. ", log.chips.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join(", "));
            }
        }
        let name = "Test keys: activate, then press keys. Tab leaves. Nothing is sent.";
        widgets::a11y(&resp, Role::Button, name, None);
        ui.ctx().accesskit_node_builder(resp.id, |node| node.set_description(intro.clone()));
        let resp = resp.on_hover_text(&intro);
        let _ = resp;
        // Its value is what was typed, so a reader's text interface has it.
        if spoken.is_empty() { spoken = "No keys tested yet.".to_owned(); }
        a11y_text(ui, Id::unique("keys-test-out"), r, Role::Status, &spoken, Some(Live::Polite));
        y = r.bottom() + 8.0;
        let i_now = Instant::now();
        let (note, progress) = match &self.keys.tester {
            Some((t, ..)) => (t.note.clone(), t.progress(i_now)),
            None => (None, None),
        };
        if let Some(f) = progress {
            widgets::progress_bar(ui, Rect::from_min_size(pos2(at.x, y), vec2(w, 3.0)), Id::unique("test-progress"), Some(f));
            y += 9.0;
        }
        // How a tap/hold key was decided, once there is one.
        if let Some(note) = note {
            y += para(ui, pos2(at.x, y), w, &note, regular(T_META), pal().ink2) + 6.0;
        }
        let safe = format!("{} always brings the mouse and keyboard back here. No rule can change it.", crate::view::chord_label("ctrl+alt+shift+esc", mac()));
        let safety_at = pos2(at.x, y);
        let safety_height = widgets::para_code(ui, safety_at, w, &safe, regular(T_META), pal().ink3);
        y += safety_height;
        a11y_text(ui, Id::unique("keys-safe"), Rect::from_min_size(safety_at, vec2(w, safety_height)), Role::Label, &safe.replace('`', ""), None);
        let _ = now;
        y - at.y
    }

    // ---------- mouse ----------

    /// The mouse's panel: the chosen button and what it should do, then the
    /// pointer and scrolling rows, the wheel and touchpad, and autoscroll
    /// and gestures, all in the panel's row style. Returns how tall it is.
    fn mouse_panel(&mut self, ui: &mut Ui, at: Pos2, w: f32) -> f32 {
        let input = self.prefs.input.clone();
        let facts = self.facts();
        let mut y = self.buttons_card(ui, at, w, &input, &facts);
        y = self.pointer_rows(ui, pos2(at.x, y + PANEL_GAP), w, &input, &facts);
        y = self.scroll_card(ui, pos2(at.x, y + 6.0), w, &input, &facts);
        let mh = self.mouse_modes_card(ui, pos2(at.x, y + 6.0), w, &input, &facts);
        y + 6.0 + mh - at.y
    }

    fn mouse_modes_card(&mut self, ui: &mut Ui, at: Pos2, width: f32, input: &Input, facts: &Facts) -> f32 {
        let card = card_begin(ui, at, width, "Autoscroll & gestures");
        let content = card.inner;
        let mut top = content.top();
        if matches!(self.keys.scope, Scope::Device(..)) {
            top += para(ui, pos2(content.left(), top), content.width(), "Autoscroll and gestures apply to every mouse during a visit, not to one device. Choose All computers, an OS, or a computer in For: to change them.", regular(T_BODY), pal().ink2);
            return card.end(ui, top);
        }
        if let Some(Ok(problems)) = &self.keys.problems {
            for problem in problems.iter().filter(|problem| problem.contains("autoscroll") || problem.contains("gesture") || problem.contains("primary left/right")) {
                top += error_line(ui, content.left(), top, content.width(), &tune::plain_problem(problem)) + 6.0;
            }
        }
        let busy = self.tuning_busy();
        let mut rows: Vec<_> = input.autoscroll.iter().filter(|row| row.app.is_none())
            .filter(|row| !matches!(row.scope, Scope::Device(..)))
            .filter(|row| !row.button.as_deref().is_some_and(tune::primary_mouse_button))
            .filter_map(|row| Some((row.scope.rank(facts)?, row))).collect();
        rows.sort_by_key(|(rank, _)| *rank);
        let mut enabled = false;
        let mut speed = 1.0;
        let mut button = "middle";
        for (_, row) in rows { enabled = row.enabled.unwrap_or(enabled); speed = row.speed.unwrap_or(speed); button = row.button.as_deref().unwrap_or(button); }
        let rect = Rect::from_min_size(pos2(content.left(), top), vec2(content.width(), 46.0));
        if switch_help(ui, rect, Id::unique("autoscroll-toggle"), "Autoscroll", "autoscroll-toggle", enabled, !busy).clicked() {
            let mut args = vec!["autoscroll".into(), if enabled { "--off".into() } else { "--on".into() }];
            args.extend(self.keys.scope.args());
            self.tune_cmds("autoscroll", "mouse-modes", vec![args], None);
        }
        top += 46.0 + 2.0;
        let instructions = format!("Click {}, then move away to scroll the other computer. Click again or press Esc to stop.", tune::mouse_button_label(button));
        let height = para(ui, pos2(content.left(), top), content.width(), &instructions, regular(T_META), pal().ink2);
        a11y_text(ui, Id::unique("autoscroll-instructions"), Rect::from_min_size(pos2(content.left(), top), vec2(content.width(), height)), Role::Label, &instructions, None);
        top += height + 4.0;
        let rect = Rect::from_min_size(pos2(content.left(), top), vec2(content.width(), 46.0));
        if let Some(value) = ui.add_enabled_ui(!busy, |ui| self.speed_row(ui, rect, "autoscroll-speed", "Autoscroll speed", speed)).inner {
            let mut args = vec!["autoscroll".into(), format!("--speed={value}")];
            args.extend(self.keys.scope.args());
            self.tune_cmds("autoscroll speed", "mouse-modes", vec![args], Some("autoscroll-speed".into()));
        }
        widgets::hairline(ui.painter(), rect.x_range(), rect.bottom());
        top += 46.0 + 12.0;
        top += para(ui, pos2(content.left(), top), content.width(), "Gestures", regular(T_BODY), pal().ink) + 6.0;
        let rect = Rect::from_min_size(pos2(content.left(), top), vec2(content.width(), 30.0));
        let preset = ui.add_enabled_ui(!busy, |ui| chip(ui, rect, Id::unique("gesture-side-swipe"), "Set up side-button desktop swipes", false, true, false)).inner;
        widgets::a11y(&preset, Role::Button, "Set up side-button desktop swipes", None);
        if preset.clicked() {
            let mut args = vec!["gestures".into(), "preset".into(), "side-swipe".into()];
            args.extend(self.keys.scope.args());
            self.tune_cmds("gestures", "mouse-modes", vec![args], None);
        }
        top += 38.0;
        let gestures: Vec<_> = input.gestures.iter().filter(|row| !matches!(row.scope, Scope::Device(..)))
            .filter(|row| !tune::primary_mouse_button(&row.button)).filter(|row| row.scope.rank(facts).is_some()).collect();
        if gestures.is_empty() {
            top += para(ui, pos2(content.left(), top), content.width(), "No gestures yet. A short click still clicks normally. Use the preset or add your own below.", regular(T_META), pal().ink2);
        }
        for row in gestures {
            let description = format!("Hold {} and swipe {} to {}{}.", tune::mouse_button_label(&row.button), row.direction, row.to.describe(), row.app.as_ref().map_or(String::new(), |app| format!(" in app {app}")));
            let height = para(ui, pos2(content.left(), top), content.width(), &description, regular(T_BODY), pal().ink);
            a11y_text(ui, Id::unique(("gesture-description", &row.button, &row.direction, row.scope.label(), &row.app)),
                Rect::from_min_size(pos2(content.left(), top), vec2(content.width(), height)), Role::Label, &description, None);
            top += height + 4.0;
            let rect = Rect::from_min_size(pos2(content.left(), top), vec2(100.0_f32.min(content.width()), 30.0));
            let remove = ui.add_enabled_ui(!busy, |ui| chip(ui, rect, Id::unique(("remove-gesture", &row.button, &row.direction, row.scope.label(), &row.app)), "Remove", false, true, false)).inner;
            widgets::a11y(&remove, Role::Button, &format!("Remove gesture: {description}"), None);
            if remove.clicked() {
                let mut args = vec!["gestures".into(), "remove".into(), row.button.clone(), row.direction.clone()];
                args.extend(row.scope.args());
                if let Some(app) = &row.app { args.push(format!("--app={app}")); }
                self.tune_cmds("the gesture", "mouse-modes", vec![args], None);
            }
            top += 38.0;
        }
        top = self.gesture_editor(ui, content.left(), top + 12.0, content.width());
        if let Some((_, error)) = self.keys.error.clone().filter(|(area, _)| *area == "mouse-modes") {
            top += error_line(ui, content.left(), top, content.width(), &error);
        }
        card.end(ui, top)
    }

    fn gesture_editor(&mut self, ui: &mut Ui, left: f32, mut top: f32, width: f32) -> f32 {
        top += para(ui, pos2(left, top), width, "Add a gesture", semibold(T_BODY), pal().ink) + 8.0;
        for (key, choices, default) in [
            ("Gesture button", &[("middle", "Middle"), ("button4", "Back"), ("button5", "Forward")][..], "button4"),
            ("Swipe direction", &[("left", "Left"), ("right", "Right"), ("up", "Up"), ("down", "Down")][..], "left"),
        ] {
            let current = self.drafts.get(key).map_or(default, String::as_str);
            let track = Rect::from_min_size(pos2(left, top), vec2(width, 28.0));
            if let Some(value) = self.settings_segment(ui, track, key, choices, current) { self.drafts.insert(key.into(), value); }
            top += 36.0;
        }
        let capture_id = Id::unique("gesture-recording");
        let mut recording = ui.ctx().data(|data| data.get_temp::<bool>(capture_id)).unwrap_or(false);
        let mut value = self.drafts.get("gesture-output").cloned().unwrap_or_default();
        let field = Rect::from_min_size(pos2(left, top), vec2(width - 86.0, 28.0));
        widgets::field(ui, field, Id::unique("gesture-output"), &mut value, "Key or shortcut", "What the gesture types", !recording, false);
        let capture = Rect::from_min_size(pos2(left + width - 80.0, top), vec2(80.0, 28.0));
        let started = button(ui, capture, Id::unique("gesture-record"), if recording { "Cancel" } else { "Record" }, Look::Quiet, true, Some("Press a key or shortcut for this gesture")).clicked();
        if started { recording = !recording; }
        if recording && !started && let Some(result) = super::settings_controls::recorded(&ui.input(|input| input.events.clone()), false) {
            if let Ok(chord) = result { value = chord; }
            recording = false;
        }
        ui.ctx().data_mut(|data| data.insert_temp(capture_id, recording));
        self.drafts.insert("gesture-output".into(), value.clone());
        top += 36.0;
        let rect = Rect::from_min_size(pos2(left, top), vec2(130.0, 28.0));
        if button(ui, rect, Id::unique("gesture-add"), "Add gesture", Look::Primary, !recording && !value.trim().is_empty() && !self.tuning_busy(), None).clicked() {
            let held = self.drafts.get("Gesture button").map_or("button4", String::as_str);
            let direction = self.drafts.get("Swipe direction").map_or("left", String::as_str);
            match gesture_args(held, direction, &value, &self.keys.scope) {
                Ok(args) => self.tune_cmds("the gesture", "mouse-modes", vec![args], None),
                Err(error) => self.keys.error = Some(("mouse-modes", error)),
            }
        }
        top + 38.0
    }

    /// The mouse on the floor, its five buttons labelled on cards (handoff
    /// 5.5) with what each does now: click a part or its card to change it.
    fn mouse_stage(&mut self, ui: &mut Ui, r: Rect, input: &Input, facts: &Facts) {
        const PARTS: [(u32, &str); 5] = [(1, "left"), (2, "right"), (3, "wheel"), (4, "back"), (5, "forward")];
        let model = self.keys.mouse_model.get_or_insert_with(|| std::sync::Arc::new(crate::gear::Model::parse(include_bytes!("../../assets/gear/mouse.omkm")).expect("the mouse model is built in"))).clone();
        let room = mouse_room(r);
        let chosen = self.keys.button.and_then(|n| PARTS.iter().find(|p| p.0 == n)).map(|p| p.1);
        let dt = ui.input(|i| i.stable_dt);
        let mut moving = self.keys.mouse_orbit.step(dt);
        let turn = self.keys.mouse_orbit;
        // Bring the selected surface toward the eye as the camera moves in.
        // A side button reads better side-on; primary buttons read from above.
        let side_focus = matches!(chosen, Some("back" | "forward"));
        let focus_yaw = tween(ui.ctx(), Id::unique("mouse-focus-yaw"), if side_focus { 0.55 } else { 0.0 }, 0.45, motion::out_cubic);
        let focus_pitch = tween(ui.ctx(), Id::unique("mouse-focus-pitch"), if side_focus { -0.20 } else if chosen.is_some() { 0.20 } else { 0.0 }, 0.45, motion::out_cubic);
        moving |= crate::gear::ModelView::get(&mut self.keys.mouse, &model, r, room, 0.0, -2.2 + turn.yaw + focus_yaw, 0.62 + turn.pitch + focus_pitch, chosen, chosen, &mut self.keys.mouse_glide, dt);
        if moving {
            ui.ctx().request_repaint();
        }
        let Some(view) = self.keys.mouse.take() else { return };
        let under_painter = ui.painter_at(r);
        let under_idx = under_painter.add(egui::Shape::Noop);
        let shown = tune::buttons_shown(input, facts);
        let busy = self.tuning_busy();
        // Clicks on the mouse itself.
        let resp = ui.interact(room, Id::unique("mouse-body"), widgets::sense(ui, Sense::CLICK | Sense::DRAG));
        let guidance = "Click a button to change it; click it again to return to the whole mouse. Drag to turn the mouse; double-click to reset its angle.";
        widgets::a11y(&resp, Role::Image, "3D mouse", None);
        ui.ctx().accesskit_node_builder(resp.id, |node| node.set_description(guidance));
        let resp = resp.on_hover_text(guidance);
        if resp.dragged() {
            self.keys.mouse_orbit.pull(resp.drag_delta(), MOUSE_TURN.0, MOUSE_TURN.1);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if resp.double_clicked() {
            self.keys.mouse_orbit.reset();
        }
        let names: Vec<&str> = PARTS.iter().map(|p| p.1).collect();
        let under = resp.hover_pos().and_then(|pos| view.hit(pos, &names)).and_then(|n| PARTS.iter().find(|p| p.1 == n).map(|p| p.0));
        let mut pick = None;
        if under.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                pick = under;
            }
        } else if resp.clicked() && self.keys.button.is_some() {
            // Tapping away from the buttons lets go of the chosen one, and
            // the view zooms back out.
            self.keys.button = None;
            self.keys.reveal = false;
        }
        let anchor = |n: u32| PARTS.iter().find(|p| p.0 == n).and_then(|(_, name)| view.parts.iter().find(|part| part.name == *name)).map(|part| part.anchor);
        let cards = mouse_cards(r);
        let mut hovered_label = None;
        let painter = ui.painter_at(r);
        // The leader lines first, so no card is crossed by another's line.
        for (n, card, left) in &cards {
            let Some(anchor) = anchor(*n) else { continue };
            let changed = shown.iter().find(|(k, ..)| k == n).is_some_and(|(_, _, ch)| *ch);
            let sel = self.keys.button == Some(*n);
            let lit = under == Some(*n) || self.keys.mouse_hover == Some(*n);
            let ex = if *left { card.right() } else { card.left() };
            let ey = card.center().y;
            let mid = ex + if *left { 14.0 } else { -14.0 };
            let line = if sel || changed { pal().accent } else { pal().tint(if lit { 150 } else { 80 }) };
            painter.line(vec![pos2(ex, ey), pos2(mid, ey), anchor], Stroke::new(if sel { 1.5 } else { 1.0 }, line));
            let (fill, ring) = if sel || changed { (pal().accent, pal().surface) } else { (pal().surface, pal().ink) };
            painter.circle(anchor, 4.5, fill, Stroke::new(1.5, ring));
        }
        for (n, card, _) in &cards {
            let name = tune::BUTTONS.iter().find(|b| b.0 == *n).map_or("Button", |b| b.1);
            let (does, changed) = shown.iter().find(|(k, ..)| k == n).map_or((tune::button_default(*n).to_owned(), false), |(_, d, ch)| (d.clone(), *ch));
            let id = Id::unique(("mouse-label", *n));
            let lresp = ui.interact(*card, id, widgets::sense(ui, Sense::click()));
            let sel = self.keys.button == Some(*n);
            let lit = lresp.hovered() || under == Some(*n);
            if lresp.hovered() {
                hovered_label = Some(*n);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let hover = tween(ui.ctx(), id.with("hover"), f32::from(lit), 0.12, motion::out_cubic);
            let radius = CornerRadius::same(12);
            for s in widgets::card_shadow(*card, radius) {
                painter.add(s);
            }
            painter.rect_filled(*card, radius, mix(pal().surface, pal().surface2_hover, hover * 0.5));
            if sel {
                // The selection: 2 points of the accent and its soft ring.
                painter.rect_stroke(*card, radius, Stroke::new(4.0, pal().ring), StrokeKind::Outside);
                painter.rect_stroke(*card, radius, Stroke::new(2.0, pal().accent), StrokeKind::Inside);
            } else {
                painter.rect_stroke(*card, radius, Stroke::new(1.0, pal().line2), StrokeKind::Inside);
            }
            widgets::focus_ring(ui, &lresp, *card, 12);
            let col = if changed { pal().accent_text } else { pal().ink };
            let tx = card.left() + 14.0;
            let room = card.width() - 28.0;
            let text = fit(&painter, &does, &semibold(T_BODY), room);
            if card.height() >= 44.0 {
                painter.text(pos2(tx, card.top() + 15.0), Align2::LEFT_CENTER, fit(&painter, name, &regular(T_CHIP), room), regular(T_CHIP), pal().ink2);
                painter.text(pos2(tx, card.top() + 32.0), Align2::LEFT_CENTER, &text, semibold(T_BODY), col);
            } else {
                // A short stage: what it does, its name on hover.
                painter.text(pos2(tx, card.center().y), Align2::LEFT_CENTER, &text, semibold(T_BODY), col);
            }
            widgets::a11y(&lresp, Role::Button, &format!("{name}: {does}. Change it"), Some(sel));
            let help = if sel { "Activate again to return to the whole mouse." } else { "Select this button to zoom in and change its action." };
            ui.ctx().accesskit_node_builder(lresp.id, |node| node.set_description(help));
            if lresp.hovered() && (sel || text != does || card.height() < 44.0) {
                lresp.clone().on_hover_text(format!("{name}: {does}. {help}"));
            }
            if lresp.clicked() && !busy {
                pick = Some(*n);
            }
        }
        if under.is_some() || hovered_label.is_some() {
            let want = under.or(hovered_label);
            if want != self.keys.mouse_hover {
                self.keys.mouse_hover = want;
                ui.ctx().request_repaint();
            }
        } else if self.keys.mouse_hover.take().is_some() {
            ui.ctx().request_repaint();
        }
        // The mouse goes under its labels' lines: into the place kept for it.
        let hl = self.keys.mouse_hover.and_then(|n| PARTS.iter().find(|p| p.0 == n)).map(|p| p.1);
        under_painter.set(under_idx, view.shapes(hl));
        if let Some(n) = pick {
            self.keys.button = if self.keys.button == Some(n) { None } else { Some(n) };
            self.keys.reveal = self.keys.button.is_some();
            self.keys.capture = None;
            self.keys.picker = None;
        }
        self.keys.mouse = Some(view);
    }

    /// The chosen button (handoff 5.5): its name, "What should it do?" and
    /// four choices (40 tall, corner 10); the open one shows how. With none
    /// chosen, how to choose one. The side buttons' quick setup follows.
    /// Returns the bottom.
    fn buttons_card(&mut self, ui: &mut Ui, at: Pos2, w: f32, input: &Input, facts: &Facts) -> f32 {
        let mut y = at.y;
        let busy = self.tuning_busy();
        if let Some(n) = self.keys.button {
            let name = tune::BUTTONS.iter().find(|b| b.0 == n).map_or("Button", |b| b.1);
            let p = ui.painter().clone();
            let close = Rect::from_center_size(pos2(at.x + w - 13.0, at.y + 13.0), vec2(26.0, 26.0));
            p.text(pos2(at.x, at.y + 11.0), Align2::LEFT_CENTER, fit(&p, name, &semibold(T_TITLE), close.left() - at.x - 6.0), semibold(T_TITLE), pal().ink);
            p.text(pos2(at.x, at.y + 31.0), Align2::LEFT_CENTER, "What should it do?", regular(T_META), pal().ink2);
            a11y_text(ui, Id::unique("button-editor-title"), Rect::from_min_size(at, vec2(w - 30.0, 40.0)), Role::Heading, &format!("{name}: what should it do?"), None);
            if widgets::icon_button(ui, close, Id::unique("button-close"), &format!("Done with {name}"), true, draw_cross).clicked() {
                self.keys.button = None;
                self.keys.capture = None;
                self.keys.picker = None;
                return self.buttons_card(ui, at, w, input, facts);
            }
            if std::mem::take(&mut self.keys.reveal) {
                ui.scroll_to_rect(Rect::from_min_size(pos2(at.x, at.y - 6.0), vec2(w, 60.0)), None);
            }
            y += 40.0 + PANEL_GAP;
            let code = keynames::button_code(n).map(|c| c as u16);
            let doing = input.specs(facts).into_iter().find(|(s, _)| Some(s.from.key) == code && !s.from.has_mods() && s.layer.is_none()).and_then(|(s, _)| match s.what {
                What::To(o) => Some(o),
                What::TapHold { .. } => None,
            });
            let now = ButtonKind::of(doing.as_ref());
            let open = self.keys.button_kind.filter(|(b, _)| *b == n).map_or(now, |(_, k)| k);
            for kind in ButtonKind::ALL {
                let row = Rect::from_min_size(pos2(at.x, y), vec2(w, 40.0));
                let id = Id::unique(("button-kind", kind.label()));
                let resp = ui.interact(row, id, if busy { Sense::hover() } else { widgets::sense(ui, Sense::click()) });
                let on = kind == open;
                let hover = tween(ui.ctx(), id.with("hover"), f32::from(resp.hovered() && !on), 0.1, motion::out_cubic);
                let radius = CornerRadius::same(10);
                let p = ui.painter();
                if on {
                    p.rect(row, radius, pal().accent_soft, Stroke::new(1.5, pal().accent), StrokeKind::Inside);
                } else {
                    p.rect(row, radius, mix(Color32::TRANSPARENT, pal().hover, hover), Stroke::new(1.0, pal().line2), StrokeKind::Inside);
                }
                let dot = pos2(row.left() + 20.0, row.center().y);
                if on {
                    p.circle_filled(dot, 8.0, pal().accent);
                    p.circle_filled(dot, 3.0, pal().switch_knob);
                } else {
                    p.circle_stroke(dot, 7.25, Stroke::new(1.5, pal().line2));
                }
                p.text(pos2(row.left() + 38.0, row.center().y), Align2::LEFT_CENTER, kind.label(), regular(T_BODY), pal().ink);
                widgets::focus_ring(ui, &resp, row, 10);
                if resp.hovered() && !busy {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let label = format!("{name}: {}", kind.label());
                let enabled = !busy;
                resp.widget_info(move || egui::WidgetInfo::selected(Role::RadioButton, enabled, on, &label));
                if resp.clicked() && !busy {
                    self.keys.capture = None;
                    self.keys.picker = None;
                    if kind == ButtonKind::AsItIs {
                        self.keys.button_kind = None;
                        if now != ButtonKind::AsItIs {
                            self.tune_cmds(&name.to_lowercase(), "buttons", vec![tune::button_args(n, None, &self.keys.scope)], None);
                        }
                    } else {
                        self.keys.button_kind = Some((n, kind));
                    }
                }
                y += 40.0 + 6.0;
                if on && kind != ButtonKind::AsItIs {
                    y = self.button_how(ui, pos2(at.x, y + 2.0), w, n, name, kind, doing.as_ref()) + 8.0;
                }
            }
            y -= 6.0;
        } else {
            y += para(ui, pos2(at.x, y), w, "Click a part of the mouse above, or its label, to change what it does.", regular(T_BODY), pal().ink2);
        }
        y += 12.0;
        // The one quick setup for buttons.
        let side = "Side buttons go back and forward";
        let on = [(4u32, "back"), (5, "forward")].iter().all(|(n, to)| {
            input.buttons.iter().any(|b| b.scope == self.keys.scope && b.from.trim() == n.to_string() && b.to == tune::To::One((*to).into()))
        });
        let cr = Rect::from_min_size(pos2(at.x, y), vec2(chip_width(ui, side, on).min(w), 30.0));
        if chip(ui, cr, Id::unique("preset-side"), side, on, !busy, false).clicked() {
            let cmds = if on {
                vec![tune::button_args(4, None, &self.keys.scope), tune::button_args(5, None, &self.keys.scope)]
            } else {
                let mut a = vec!["buttons".to_owned(), "preset".into()];
                a.extend(self.keys.scope.args());
                a.extend(["--".into(), "side-back-forward".into()]);
                vec![a]
            };
            self.tune_cmds("the side buttons", "buttons", cmds, None);
        }
        y += 30.0;
        if input.local_rules && cfg!(any(windows, target_os = "macos")) {
            for rule in input.buttons.iter().filter(|rule| rule.scope.includes_local_os(one_mk_proto::os::local())) {
                y += 6.0;
                y += para(ui, pos2(at.x, y), w, &format!("{} → {} · Also here", rule.from, rule.to.describe()), regular(T_META), pal().ink2);
            }
        }
        if let Some((_, e)) = self.keys.error.clone().filter(|(a, _)| *a == "buttons") {
            y += 6.0 + error_line(ui, at.x, y + 6.0, w, &e);
        }
        y
    }

    /// Under the open choice: how ("Press a key or shortcut" or "Choose
    /// from a list"; the computers to switch to; the command). Its bottom.
    #[allow(clippy::too_many_arguments)]
    fn button_how(&mut self, ui: &mut Ui, at: Pos2, w: f32, n: u32, name: &str, kind: ButtonKind, doing: Option<&one_mk_proto::remap::Output>) -> f32 {
        let busy = self.tuning_busy();
        let (left, mut y) = (at.x, at.y);
        match kind {
            ButtonKind::Key => match (self.keys.capture, self.keys.picker.clone()) {
                (Some((Want::Button, _)), _) => y = self.capture_box(ui, left, y, w, name, Want::Button),
                (_, Some((Want::Button, q))) => y = self.picker(ui, left, y, w, Want::Button, q, 6),
                _ => {
                    let labels = ["Press a key or shortcut", "Choose from a list"];
                    // Side by side, equal, when both fit; else one above the other.
                    let side = labels.iter().all(|l| button_width(ui, l) <= (w - 8.0) / 2.0);
                    let rects: Vec<Rect> = if side {
                        let bw = (w - 8.0) / 2.0;
                        vec![Rect::from_min_size(pos2(left, y), vec2(bw, 36.0)), Rect::from_min_size(pos2(left + bw + 8.0, y), vec2(bw, 36.0))]
                    } else {
                        vec![Rect::from_min_size(pos2(left, y), vec2(w, 36.0)), Rect::from_min_size(pos2(left, y + 44.0), vec2(w, 36.0))]
                    };
                    let bottom = rects[1].bottom();
                    for (k, r) in rects.iter().enumerate() {
                        let look = if k == 0 { Look::Primary } else { Look::Quiet };
                        let full = [format!("{name}: press the key or shortcut it gives"), format!("{name}: choose what it does from a list")];
                        if widgets::content_button(ui, *r, Id::unique(("button-act", k)), labels[k], look, !busy, Some(&full[k])).clicked() {
                            match k {
                                0 => self.keys.capture = Some((Want::Button, Capture::default())),
                                _ => self.keys.picker = Some((Want::Button, String::new())),
                            }
                        }
                    }
                    y = bottom;
                }
            },
            ButtonKind::Switch => {
                // The next computer, back here, or one by name.
                let current = doing.map(|o| o.to_string()).unwrap_or_default();
                let mut options = vec![("Switch to the next computer".to_owned(), "switch next".to_owned()), ("Come back to this computer".to_owned(), "switch here".to_owned())];
                options.extend(self.scope_peers().into_iter().map(|p| (p.name.clone(), format!("switch {}", p.name))));
                let widths: Vec<f32> = options.iter().map(|(l, _)| chip_width(ui, l, false).min(w)).collect();
                let (rects, bottom) = quick_setup_rects(w, 0.0, &widths);
                for ((label, value), rect) in options.iter().zip(rects) {
                    let rect = rect.translate(vec2(left, y - 38.0));
                    let on = current == *value;
                    if chip(ui, rect, Id::unique(("button-switch", value.as_str())), label, on, !busy, false).clicked() && !on {
                        self.choose(Want::Button, value.clone());
                    }
                }
                y += bottom - 38.0;
            }
            ButtonKind::Run => {
                let current = match doing {
                    Some(one_mk_proto::remap::Output::Action(one_mk_proto::remap::Action::Run(cmd))) => cmd.clone(),
                    _ => String::new(),
                };
                let fr = Rect::from_min_size(pos2(left, y), vec2(w, 34.0));
                if let Some(cmd) = self.edit_field(ui, fr, "button-run", &current, "Type a command, then Enter", &format!("{name}: a command to run when it's pressed (Enter saves)"), !busy, true)
                    && !cmd.is_empty()
                {
                    self.choose(Want::Button, format!("run: {cmd}"));
                }
                y += 34.0;
            }
            ButtonKind::AsItIs => {}
        }
        let _ = n;
        y
    }

    /// A speed (handoff 5.5: "Pointer speed 1.0×"): the label, a slider
    /// (even by ratio, 1× in the middle) and the number in mono.
    fn speed_row(&mut self, ui: &mut Ui, row: Rect, key: &'static str, label: &str, value: f64) -> Option<f64> {
        // One line: the caption stays in the "i" (no room beside a slider).
        let lw = label_help_inline(ui, Rect::from_center_size(row.center(), vec2(row.width(), 36.0)), Id::unique(("speed-label", key)), label, key, true);
        let shown = self.keys.drafts.get(key).copied().unwrap_or(value);
        let fw = 46.0;
        let suffix = text_width(ui, "×", mono(T_BODY)) + 4.0;
        let fr = Rect::from_min_size(pos2(row.right() - fw - suffix, row.center().y - 14.0), vec2(fw, 28.0));
        let sr = Rect::from_min_max(pos2(row.left() + lw - 6.0, row.top()), pos2(fr.left() - 8.0, row.bottom()));
        let mut out = None;
        if sr.width() >= 48.0 {
            let (t, apply) = slider(ui, sr, Id::unique(("slider", key)), tune::speed_to_t(shown), label, shown);
            if let Some(t) = t {
                let x = tune::t_to_speed(t);
                self.keys.drafts.insert(key, x);
                if apply {
                    out = Some(x);
                }
            }
        }
        ui.painter().text(pos2(row.right(), row.center().y), Align2::RIGHT_CENTER, "×", mono(T_BODY), pal().ink2);
        let cur = tune::speed_text(shown).trim_end_matches('×').to_owned();
        if let Some(text) = self.edit_field(ui, fr, key, &cur, "1", &format!("{label}, from 0.1 to 10 times"), true, true) {
            match tune::parse_speed(&text) {
                Ok(x) => {
                    self.keys.drafts.insert(key, x);
                    out = Some(x);
                }
                Err(e) => self.keys.error = Some((if key.starts_with("pointer") { "pointer" } else if key.starts_with("autoscroll") { "mouse-modes" } else { "scroll" }, e)),
            }
        }
        out
    }

    /// The handoff's rows after a hairline: Pointer speed, (Acceleration,
    /// when this computer can), Scroll speed, Natural scrolling, Shift
    /// scrolls sideways. Returns the bottom.
    fn pointer_rows(&mut self, ui: &mut Ui, at: Pos2, w: f32, input: &Input, facts: &Facts) -> f32 {
        widgets::hairline(ui.painter(), egui::Rangef::new(at.x, at.x + w), at.y);
        let mut y = at.y + 6.0;
        let row = |y: f32| Rect::from_min_size(pos2(at.x, y), vec2(w, 46.0));
        let line = |ui: &Ui, y: f32| widgets::hairline(ui.painter(), egui::Rangef::new(at.x, at.x + w), y);
        let (speed, accel) = tune::pointer_shown(input, facts);
        if let Some(x) = self.speed_row(ui, row(y), "pointer-speed", "Pointer speed", speed) {
            self.tune_cmds("the pointer speed", "pointer", vec![tune::pointer_args(Some(x), None, &self.keys.scope)], Some("pointer-speed".into()));
        }
        y += 46.0;
        if tune::RAW_MOTION {
            line(ui, y);
            let r = row(y);
            let lw = label_help_inline(ui, Rect::from_center_size(r.center(), vec2(r.width(), 36.0)), Id::unique("accel-label"), "Acceleration", "pointer-accel", true);
            let track = Rect::from_min_max(pos2(r.left() + lw, r.center().y - 15.0), pos2(r.right(), r.center().y + 15.0));
            let labels = ["Like here", "Off", "Curve"];
            let values = ["system", "off", "curve"];
            let can = tune::RAW_MOTION;
            let pills = Pills {
                id: Id::unique("pointer-accel"),
                track,
                cells: Pills::even(track, 3, 3.0),
                labels: &labels,
                names: vec!["Acceleration: as this computer does it".into(), "Acceleration off".into(), "Acceleration curve".into()],
                chosen: values.iter().position(|v| *v == accel),
                enabled: vec![true, can, can],
                role: Role::RadioButton,
                font: T_META,
            };
            if let Some(k) = pills.show(ui, self.arrowed) {
                self.tune_cmds("the acceleration", "pointer", vec![tune::pointer_args(None, Some(values[k]), &self.keys.scope)], None);
            }
            y += 46.0;
        }
        if let Some((_, e)) = self.keys.error.clone().filter(|(a, _)| *a == "pointer") {
            y += 6.0 + error_line(ui, at.x, y + 6.0, w, &e) + 6.0;
        }
        let dev = self.keys.device;
        let s = tune::scroll_shown(input, facts, dev);
        let mut change = None;
        line(ui, y);
        if let Some(x) = self.speed_row(ui, row(y), "scroll-speed", "Scroll speed", s.speed) {
            change = Some(ScrollChange::Speed(x));
        }
        y += 46.0;
        line(ui, y);
        // Natural scrolling: the choice in force, a menu of the four.
        let r = row(y);
        let lw = label_help_inline(ui, Rect::from_center_size(r.center(), vec2(r.width(), 36.0)), Id::unique("natural-label"), "Natural scrolling", "scroll-natural", true);
        let nat = ["As here", "Match each", "Always", "Never"];
        let nat_values: [Option<&'static str>; 4] = [None, Some("auto"), Some("on"), Some("off")];
        let names = [
            "Natural scrolling: as on this computer",
            "Natural scrolling: match the computer I'm on (natural on Macs, traditional on PCs)",
            "Natural scrolling: always",
            "Natural scrolling: never",
        ];
        let at_nat = nat_values.iter().position(|v| *v == s.natural.as_deref()).unwrap_or(0);
        let vw = (text_width(ui, nat[at_nat], regular(T_BODY)) + 34.0).min(r.width() - lw + 10.0);
        let vr = Rect::from_min_size(pos2(r.right() - vw + 8.0, r.center().y - 15.0), vec2(vw, 30.0));
        let resp = ui.interact(vr, Id::unique("scroll-natural"), widgets::sense(ui, Sense::click()));
        let hover = tween(ui.ctx(), resp.id.with("hover"), f32::from(resp.hovered()), 0.1, motion::out_cubic);
        let p = ui.painter();
        p.rect_filled(vr, CornerRadius::same(8), mix(Color32::TRANSPARENT, pal().hover, hover));
        p.text(pos2(vr.right() - 26.0, vr.center().y), Align2::RIGHT_CENTER, nat[at_nat], regular(T_BODY), pal().ink2);
        let c = pos2(vr.right() - 14.0, vr.center().y);
        p.add(egui::Shape::line(vec![c + vec2(-4.0, -2.0), c + vec2(0.0, 2.0), c + vec2(4.0, -2.0)], Stroke::new(1.5, pal().ink2)));
        widgets::focus_ring(ui, &resp, vr, 8);
        widgets::a11y(&resp, Role::ComboBox, &format!("{}. Choose", names[at_nat]), None);
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        widgets::debug_open_menu(ui, &resp, "natural");
        egui::Popup::menu(&resp).width(240.0).align(egui::RectAlign::BOTTOM_END).show(|ui| {
            for (k, label) in nat.iter().enumerate() {
                if widgets::menu_item(ui, 240.0, widgets::MenuItem::choice(label, names[k], k == at_nat)).clicked() {
                    if k != at_nat {
                        change = Some(ScrollChange::Natural(nat_values[k]));
                    }
                    ui.close();
                }
            }
        });
        y += 46.0;
        let explain = match s.natural.as_deref() {
            Some("auto") => "Match the computer I'm on: natural on Macs, traditional on Windows and Linux.",
            Some("on") => "Content follows your fingers on every computer.",
            Some("off") => "Content moves against your fingers on every computer.",
            _ => "Scrolling feels on the other computers as it does here.",
        };
        y += para(ui, pos2(at.x, y - 6.0), w, explain, regular(T_META), pal().ink3) + 4.0;
        line(ui, y);
        if switch_help(ui, row(y), Id::unique("scroll-shift"), "Shift scrolls sideways", "scroll-shift", s.shift_sideways, true).clicked() {
            change = Some(ScrollChange::Modifier("shift", if s.shift_sideways { "unchanged" } else { "horizontal" }));
        }
        y += 46.0;
        if let Some(ch) = change {
            let key = matches!(ch, ScrollChange::Speed(_)).then(|| "scroll-speed".to_owned());
            let cmds = tune::scroll_commands(input, &self.keys.scope, dev, &ch);
            self.tune_cmds("scrolling", "scroll", cmds, key);
        }
        y
    }

    /// The rest of scrolling, in the same row style: which devices the
    /// scroll settings are for, steps, reversing, and Alt for 5×.
    fn scroll_card(&mut self, ui: &mut Ui, at: Pos2, w: f32, input: &Input, facts: &Facts) -> f32 {
        let card = card_begin(ui, at, w, "Scrolling");
        let c = card.inner;
        let mut y = c.top() - 6.0;
        let dev = self.keys.device;
        let s = tune::scroll_shown(input, facts, dev);
        let row = |y: f32| Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), 46.0));
        let line = |ui: &Ui, y: f32| widgets::hairline(ui.painter(), c.x_range(), y);
        let seg = |ui: &mut Ui, app: &App, r: Rect, id: &str, label: &str, labels: &[&str], names: Vec<String>, chosen: Option<usize>| -> Option<usize> {
            // A narrow panel: the label above, the choices across the width.
            let track = if r.height() > 50.0 {
                label_help_inline(ui, Rect::from_min_size(r.min, vec2(r.width(), 34.0)), Id::unique((id, "label")), label, id, true);
                Rect::from_min_max(pos2(r.left(), r.top() + 34.0), pos2(r.right(), r.top() + 64.0))
            } else {
                let lw = label_help_inline(ui, Rect::from_center_size(r.center(), vec2(r.width(), 36.0)), Id::unique((id, "label")), label, id, true);
                Rect::from_min_max(pos2(r.left() + lw, r.center().y - 15.0), pos2(r.right(), r.center().y + 15.0))
            };
            let n = labels.len();
            Pills { id: Id::unique(id), track, cells: Pills::even(track, n, 3.0), labels, names, chosen, enabled: vec![true; n], role: Role::RadioButton, font: T_META }.show(ui, app.arrowed)
        };
        let seg_h = if c.width() < 360.0 { 72.0 } else { 46.0 };
        let seg_row = |y: f32| Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), seg_h));
        let devs = ["Both", "Wheel", "Touchpad"];
        let chosen = Some(match dev {
            None => 0,
            Some(Device::Wheel) => 1,
            Some(Device::Touchpad) => 2,
        });
        let names = vec!["Scroll settings for: mouse wheels and touchpads".into(), "Scroll settings for: mouse wheels".into(), "Scroll settings for: touchpads".into()];
        if let Some(k) = seg(ui, self, seg_row(y), "scroll-device", "Settings for", &devs, names, chosen) {
            self.keys.device = [None, Some(Device::Wheel), Some(Device::Touchpad)][k];
        }
        y += seg_h;
        line(ui, y);
        let mut change = None;
        let modes = ["As it comes", "Smooth", "Notched"];
        let mode_values = ["auto", "smooth", "discrete"];
        let names = vec!["Steps: as the device sends them".into(), "Steps: smooth (notches become pixels)".into(), "Steps: whole notches (lines)".into()];
        if let Some(k) = seg(ui, self, seg_row(y), "scroll-mode", "Steps", &modes, names, mode_values.iter().position(|v| *v == s.mode)) {
            change = Some(ScrollChange::Mode(mode_values[k]));
        }
        y += seg_h;
        line(ui, y);
        if switch_help(ui, row(y), Id::unique("scroll-rev-v"), "Reverse up and down", "scroll-rev-v", s.reverse_vertical, true).clicked() {
            change = Some(ScrollChange::ReverseVertical(!s.reverse_vertical));
        }
        y += 46.0;
        line(ui, y);
        if switch_help(ui, row(y), Id::unique("scroll-rev-h"), "Reverse left and right", "scroll-rev-h", s.reverse_horizontal, true).clicked() {
            change = Some(ScrollChange::ReverseHorizontal(!s.reverse_horizontal));
        }
        y += 46.0;
        line(ui, y);
        let alt = if mac() { "Option" } else { "Alt" };
        if switch_help(ui, row(y), Id::unique("scroll-alt"), &format!("Hold {alt} to scroll 5× faster"), "scroll-alt", s.alt_faster, true).clicked() {
            change = Some(ScrollChange::Modifier("alt", if s.alt_faster { "unchanged" } else { "x5" }));
        }
        y += 46.0;
        if let Some(ch) = change {
            let key = matches!(ch, ScrollChange::Speed(_)).then(|| "scroll-speed".to_owned());
            let cmds = tune::scroll_commands(input, &self.keys.scope, dev, &ch);
            self.tune_cmds("scrolling", "scroll", cmds, key);
        }
        if let Some((_, e)) = self.keys.error.clone().filter(|(a, _)| *a == "scroll") {
            y += 6.0 + error_line(ui, c.left(), y + 6.0, c.width(), &e);
        }
        let _ = card;
        y
    }
}

/// Whether a rule with this scope can apply to this computer.
fn scope_covers(scope: &Scope, peer: &tune::Peer) -> bool {
    match scope {
        // The keyboard it is typed on says nothing about the computer.
        Scope::Device(_, on) => scope_covers(on, peer),
        scope => scope.rank(&Facts { name: peer.name.clone(), id: peer.id.clone(), os: peer.os, device: None }).is_some(),
    }
}
