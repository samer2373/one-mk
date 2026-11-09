//! Fonts, colours and the type scale: Inter for words, JetBrains Mono for
//! numbers, Unbounded only for the wordmark. Two complete themes, Light and Dark,
//! each a [`Palette`] ported from the design tokens; every colour the window
//! paints comes from the active palette ([`pal`]), or, for what looks the
//! same in both themes (the 3D devices, the screens' wallpapers, a mock of
//! macOS's own window), from the fixed colours at the end of this part.
//! Nothing outside this file writes a colour down.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Painter, Stroke};
use std::cell::Cell;
use std::sync::Arc;

/// Light or dark: which palette the window is painted with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Theme {
    Light,
    #[default]
    Dark,
}

/// Settings → General → Appearance: follow the system, or one theme.
/// `[window] appearance` in `settings.toml`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    /// As written in `settings.toml` (anything else reads as System).
    pub fn parse(text: &str) -> Appearance {
        match text.trim().to_ascii_lowercase().as_str() {
            "light" => Appearance::Light,
            "dark" => Appearance::Dark,
            _ => Appearance::System,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Appearance::System => "System",
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }

    /// The theme to paint with, given what the system prefers (if known).
    pub fn theme(self, system: Option<Theme>) -> Theme {
        match self {
            Appearance::Light => Theme::Light,
            Appearance::Dark => Theme::Dark,
            Appearance::System => system.unwrap_or_default(),
        }
    }
}

/// Defines [`Palette`] once: its colours, its numbers, and how two
/// palettes blend (the 200 ms cross-fade when the theme changes).
macro_rules! palette {
    ($( $(#[$cm:meta])* $c:ident ),* ; $( $(#[$fm:meta])* $f:ident ),* $(,)?) => {
        /// Every colour (premultiplied, as egui's are) and the few numbers a
        /// theme needs. The first group are the design tokens, as named in
        /// the handoff's `tokens.json`; the rest are worked out from them for
        /// the parts of the window that have no token of their own yet.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Palette {
            pub theme: Theme,
            $( $(#[$cm])* pub $c: Color32, )*
            $( $(#[$fm])* pub $f: f32, )*
        }

        impl Palette {
            /// `a` blended toward `b` by `t` (0..1); the theme is `b`'s once past half.
            pub fn lerp(a: &Palette, b: &Palette, t: f32) -> Palette {
                if t <= 0.0 {
                    return *a;
                }
                if t >= 1.0 {
                    return *b;
                }
                Palette {
                    theme: if t < 0.5 { a.theme } else { b.theme },
                    $( $c: mix(a.$c, b.$c, t), )*
                    $( $f: a.$f + (b.$f - a.$f) * t, )*
                }
            }

            /// Every colour, by name (for the completeness tests).
            #[cfg(test)]
            pub fn colours(&self) -> Vec<(&'static str, Color32)> {
                vec![$( (stringify!($c), self.$c), )*]
            }
        }
    };
}

palette! {
    // ---- the tokens ----
    /// The window's ground, and the 3D scenes' floor area.
    bg,
    /// Panels, cards, popovers, sheets.
    surface,
    /// Quiet buttons, a segmented control's track, the chosen tab, key chips.
    surface2,
    /// Hairlines.
    line,
    /// Field and chip borders.
    line2,
    /// Text: primary, secondary, hints.
    ink, ink2, ink3,
    /// The primary action, switch on, selection, focus, progress.
    accent,
    /// A chosen list row; changed-key text and links on it.
    accent_soft, accent_text,
    /// Text and marks on the accent.
    on_accent,
    /// Status, always with a shape: ● ok, ◆ warn, ■ danger.
    ok, warn, warn_soft, danger,
    /// The chosen segment's capsule.
    seg_on,
    /// A switch's track when off, and its knob.
    switch_off, switch_knob,
    /// The soft ellipse under the 3D objects.
    floor,
    /// A faint grid (plans).
    grid,
    /// The keyboard focus ring.
    focus,
    /// What shadows are made of (opaque; [`Palette::shadow`] sets how much).
    shadow_ink,

    // ---- worked out from the tokens ----
    /// The sidebar: a step off the ground.
    sidebar,
    /// A row or control under the pointer: an overlay.
    hover,
    /// Between ink2 and ink: a hovered label.
    ink_hover,
    /// The accent under the pointer and pressed.
    accent_hover, accent_press,
    /// `surface2` under the pointer (the `line2` overlay).
    surface2_hover,
    /// A recess (a disabled control, a well): an overlay.
    well,
    /// A field's edge under the pointer.
    field_edge_lit,
    /// An off switch under the pointer.
    switch_off_lit,
    /// The outline of a drawn (2D) key.
    key_edge,
    /// The command palette's glass, top and bottom (the only glass).
    glass_top, glass,
    /// The room on a see-through window.
    room_tint,
    /// Light catching a glass edge.
    glass_edge,
    /// The soft ring outside a selection (accent at 18 %).
    ring,
    /// Lines and marks drawn on the 3D floor (a dashed slot, a hover outline):
    /// white on the dark floor, ink on the light one. See [`Palette::tint`].
    tint_ink,
    /// What pressing moves a fill toward.
    press_toward;
    /// How far pressing moves a fill toward `press_toward`.
    press_amount,
    /// How strong shadows are: 1 is the dark theme's, as before.
    shadow_k,
}

/// `a` toward `b` by `t`, premultiplied, as the controls blend.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()), f(a.a(), b.a()))
}

/// `fg` (premultiplied) laid over opaque `bg`: what the eye sees.
pub fn over(fg: Color32, bg: Color32) -> Color32 {
    let a = 1.0 - f32::from(fg.a()) / 255.0;
    let f = |x: u8, y: u8| (f32::from(x) + f32::from(y) * a).round().min(255.0) as u8;
    Color32::from_rgb(f(fg.r(), bg.r()), f(fg.g(), bg.g()), f(fg.b(), bg.b()))
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// `hex` at `alpha` (0..1), as CSS's rgba() writes it.
const fn rgba(hex: u32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied_const((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, (alpha * 255.0 + 0.5) as u8)
}

impl Palette {
    /// Daylight: warm paper, white surfaces, ink text, cobalt.
    pub fn light() -> Palette {
        let (bg, surface, surface2, ink, ink2, ink3) = (rgb(0xF4F3F0), rgb(0xFFFFFF), rgb(0xEFEEEA), rgb(0x16171A), rgb(0x6B6D73), rgb(0x8E9096));
        let (line, line2, accent) = (rgba(0x16171A, 0.08), rgba(0x16171A, 0.14), rgb(0x2F54EB));
        let black = rgb(0x000000);
        Palette {
            theme: Theme::Light,
            bg,
            surface,
            surface2,
            line,
            line2,
            ink,
            ink2,
            ink3,
            accent,
            accent_soft: rgb(0xE9EDFF),
            accent_text: rgb(0x2443C9),
            on_accent: rgb(0xFFFFFF),
            ok: rgb(0x1F9D63),
            warn: rgb(0xC27A0E),
            warn_soft: rgb(0xFBF2E2),
            danger: rgb(0xC0392B),
            seg_on: rgb(0xFFFFFF),
            switch_off: rgb(0xD9D7D1),
            switch_knob: rgb(0xFFFFFF),
            floor: rgba(0x16171A, 0.13),
            grid: rgba(0x16171A, 0.07),
            focus: rgb(0x2F54EB),
            shadow_ink: ink,
            sidebar: mix(bg, surface, 0.45),
            hover: rgba(0x16171A, 0.05),
            ink_hover: mix(ink2, ink, 0.55),
            accent_hover: mix(accent, black, 0.06),
            accent_press: mix(accent, black, 0.10),
            surface2_hover: over(line2, surface2),
            well: rgba(0x16171A, 0.04),
            field_edge_lit: rgba(0x16171A, 0.30),
            switch_off_lit: mix(rgb(0xD9D7D1), ink, 0.06),
            key_edge: ink3,
            glass_top: rgba(0xFFFFFF, 0.96),
            glass: rgba(0xFFFFFF, 0.96),
            room_tint: rgba(0xF4F3F0, 0.88),
            glass_edge: rgba(0xFFFFFF, 0.7),
            ring: rgba(0x2F54EB, 0.18),
            tint_ink: ink,
            press_toward: ink,
            press_amount: 0.10,
            shadow_k: 0.35,
        }
    }

    /// Night: near-black ground, graphite surfaces, light ink, cobalt.
    pub fn dark() -> Palette {
        let (bg, surface, surface2, ink, ink2, ink3) = (rgb(0x111214), rgb(0x1A1B1F), rgb(0x25262B), rgb(0xEDEDEB), rgb(0x9A9CA3), rgb(0x75777E));
        let (line, line2, accent) = (rgba(0xFFFFFF, 0.07), rgba(0xFFFFFF, 0.12), rgb(0x4D6DFF));
        let (black, white) = (rgb(0x000000), rgb(0xFFFFFF));
        Palette {
            theme: Theme::Dark,
            bg,
            surface,
            surface2,
            line,
            line2,
            ink,
            ink2,
            ink3,
            accent,
            accent_soft: rgb(0x1D2445),
            accent_text: rgb(0x8EA4FF),
            on_accent: white,
            ok: rgb(0x3CC985),
            warn: rgb(0xE9A23B),
            warn_soft: rgb(0x2A2216),
            danger: rgb(0xF0675A),
            seg_on: rgb(0x34353B),
            switch_off: rgb(0x3A3B41),
            switch_knob: white,
            floor: rgba(0x000000, 0.55),
            grid: rgba(0xFFFFFF, 0.06),
            focus: rgb(0x7E97FF),
            shadow_ink: black,
            sidebar: mix(bg, surface, 0.45),
            hover: rgba(0xFFFFFF, 0.07),
            ink_hover: mix(ink2, ink, 0.55),
            accent_hover: mix(accent, black, 0.06),
            accent_press: mix(accent, black, 0.10),
            surface2_hover: over(line2, surface2),
            well: rgba(0xFFFFFF, 0.05),
            field_edge_lit: rgba(0xFFFFFF, 0.28),
            switch_off_lit: mix(rgb(0x3A3B41), ink, 0.08),
            key_edge: ink3,
            glass_top: rgba(0x1A1B1F, 0.96),
            glass: rgba(0x1A1B1F, 0.96),
            room_tint: rgba(0x111214, 0.88),
            glass_edge: rgba(0xFFFFFF, 0.22),
            ring: rgba(0x4D6DFF, 0.18),
            tint_ink: white,
            press_toward: black,
            press_amount: 0.88,
            shadow_k: 1.0,
        }
    }

    pub fn of(theme: Theme) -> Palette {
        match theme {
            Theme::Light => Palette::light(),
            Theme::Dark => Palette::dark(),
        }
    }

    /// A shadow at `alpha` as the dark theme draws it, made as strong as
    /// this theme's shadows are (the light theme's are much softer).
    pub fn shadow(&self, alpha: u8) -> Color32 {
        self.shadow_ink.gamma_multiply(f32::from(alpha) / 255.0 * self.shadow_k)
    }

    /// The floor's line colour at `alpha`: white on the dark floor, ink on
    /// the light one.
    pub fn tint(&self, alpha: u8) -> Color32 {
        self.tint_ink.gamma_multiply(f32::from(alpha) / 255.0)
    }

    /// `fill` as it looks while pressed (`t` 0..1).
    pub fn pressed(&self, fill: Color32, t: f32) -> Color32 {
        mix(fill, self.press_toward, self.press_amount * t)
    }
}

thread_local! {
    /// The palette being painted with (the window paints on one thread;
    /// tests each get their own).
    static ACTIVE: Cell<Option<Palette>> = const { Cell::new(None) };
}

/// The active palette: Dark until a theme is set.
pub fn pal() -> Palette {
    ACTIVE.with(|active| active.get()).unwrap_or_else(|| {
        let dark = Palette::dark();
        ACTIVE.with(|active| active.set(Some(dark)));
        dark
    })
}

/// Paints from now on with `palette` (and egui's own widgets too, when a
/// context is given).
pub fn set_palette(palette: Palette, ctx: Option<&egui::Context>) {
    ACTIVE.with(|active| active.set(Some(palette)));
    if let Some(ctx) = ctx {
        apply_visuals(ctx);
    }
}

/// Follows the Appearance setting (and, set to System, the system's own
/// light or dark), cross-fading 200 ms between themes; with reduced motion
/// the change is immediate.
pub struct Themer {
    shown: Option<Theme>,
    from: Option<Palette>,
    since: f64,
    /// Linux: the desktop's colour scheme, read at start and whenever the
    /// window comes to the front (winit doesn't report it there).
    desktop: Arc<std::sync::Mutex<Option<Theme>>>,
    focused: bool,
}

impl Default for Themer {
    fn default() -> Self {
        Themer { shown: None, from: None, since: 0.0, desktop: Arc::default(), focused: false }
    }
}

/// How long the theme cross-fade takes.
pub const THEME_FADE: f32 = 0.2;

impl Themer {
    /// The theme `appearance` asks for now.
    pub fn wanted(&mut self, ctx: &egui::Context, appearance: Appearance) -> Theme {
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        let appearance = std::env::var("ONE_MK_APP_THEME").ok().map_or(appearance, |v| Appearance::parse(&v));
        if appearance != Appearance::System {
            return appearance.theme(None);
        }
        let focused = ctx.input(|i| i.focused);
        if cfg!(target_os = "linux") && (self.shown.is_none() || (focused && !self.focused)) {
            self.ask_desktop(ctx);
        }
        self.focused = focused;
        let desktop = *self.desktop.lock().unwrap_or_else(|e| e.into_inner());
        let system = ctx.system_theme().map(|t| if t == egui::Theme::Light { Theme::Light } else { Theme::Dark });
        appearance.theme(system.or(desktop))
    }

    fn ask_desktop(&self, ctx: &egui::Context) {
        let (slot, ctx) = (self.desktop.clone(), ctx.clone());
        std::thread::spawn(move || {
            let read = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "color-scheme"]).output();
            let theme = read.ok().filter(|o| o.status.success()).and_then(|o| {
                let said = String::from_utf8_lossy(&o.stdout);
                if said.contains("dark") { Some(Theme::Dark) } else if said.contains("light") || said.contains("default") { Some(Theme::Light) } else { None }
            });
            let mut slot = slot.lock().unwrap_or_else(|e| e.into_inner());
            if *slot != theme {
                *slot = theme;
                ctx.request_repaint();
            }
        });
    }

    /// Once a frame, before anything is painted: switches (or keeps
    /// fading) to the theme asked for. True while it is still fading.
    pub fn frame(&mut self, ctx: &egui::Context, appearance: Appearance, now: f64) -> bool {
        let want = self.wanted(ctx, appearance);
        // Development aid: ONE_MK_APP_THEME_FLIP=secs turns to the other
        // theme that far in (to see the cross-fade).
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        let want = match std::env::var("ONE_MK_APP_THEME_FLIP").ok().and_then(|v| v.parse::<f64>().ok()) {
            Some(at) if now >= at => if want == Theme::Light { Theme::Dark } else { Theme::Light },
            Some(at) => {
                ctx.request_repaint_after_secs((at - now) as f32);
                want
            }
            None => want,
        };
        if self.shown.is_none() {
            self.shown = Some(want);
            set_palette(Palette::of(want), Some(ctx));
            return false;
        }
        if self.shown != Some(want) {
            self.shown = Some(want);
            self.from = Some(pal());
            self.since = now;
        }
        let Some(from) = self.from else { return false };
        let secs = crate::motion::fade(THEME_FADE);
        let t = if secs <= 0.0 { 1.0 } else { ((now - self.since) as f32 / secs).clamp(0.0, 1.0) };
        let to = Palette::of(want);
        if t >= 1.0 {
            self.from = None;
            set_palette(to, Some(ctx));
            return false;
        }
        set_palette(Palette::lerp(&from, &to, crate::motion::out_cubic(t)), Some(ctx));
        ctx.request_repaint();
        true
    }

}

// ---- Fixed colours: the same in both themes ----

/// Images drawn as they are (no tint).
pub const UNTINTED: Color32 = Color32::WHITE;
/// Words inside the 3D screens, on their wallpapers: light in both themes.
pub const SCREEN_INK: Color32 = rgb(0xEDEFF2);
pub const SCREEN_INK2: Color32 = rgb(0x9DA6B1);
pub const SCREEN_INK_SUB: Color32 = rgb(0xC4CAD2);
/// The plate the pairing code sits on, inside a screen.
pub const SCREEN_PLATE: Color32 = rgb(0x0E1115);
/// A screen's thin frame, and the pointer's outline on it.
pub const SCREEN_EDGE: Color32 = rgb(0x30353E);
pub const POINTER_EDGE: Color32 = rgb(0x111111);
/// The pointer on a screen.
pub const POINTER: Color32 = Color32::WHITE;
/// A monitor's bezel in the wallpaper preview: near-black in both themes.
pub const BEZEL: Color32 = rgb(0x0C0D0F);
/// The 3D devices' materials (hex, as the meshes take them).
pub const STAND: u32 = 0x8E949D;
pub const MONITOR_BODY: u32 = 0x2B2F36;
pub const SCREEN_OFF: u32 = 0x0B0D10;
pub const LAPTOP_BODY: u32 = 0xAEB3BB;
/// A laptop's keyboard well and its touchpad's sheen.
pub const LAPTOP_WELL: Color32 = rgb(0x14161A);
/// The 3D keyboard: case, its top plate, keycaps, modifier keycaps.
pub const KEY_CASE: Color32 = rgb(0x3A3E45);
pub const KEY_PLATE: Color32 = rgb(0x1C1E22);
pub const KEY_CAP: Color32 = rgb(0x34373D);
pub const KEY_CAP_MOD: Color32 = rgb(0x55524D);
/// Legends printed on the (dark) keycaps, and a changed key's new legend.
pub const KEY_LEGEND: Color32 = rgb(0xC6CBD3);
pub const KEY_LEGEND_MOD: Color32 = rgb(0xD9D3C8);
pub const KEY_LEGEND_CHANGED: Color32 = rgb(0x8EA4FF);
/// The mouse model's shell (shaded by the light).
pub const MOUSE_SHELL: Color32 = Color32::GRAY;
/// Each computer's wallpaper on the desk: a light colour and a deep one,
/// as desktop pictures are (dusk violet, lagoon, ember, ocean); deep
/// enough that white names on them stay readable. Low chroma, about equal
/// lightness; this computer's is `ME_WALL`.
pub const WALLS: [[u32; 2]; 4] = [[0x7B68E0, 0x191338], [0x2E9F96, 0x08262D], [0xD9714A, 0x34101E], [0x3D7FE6, 0x0A1838]];
pub const ME_WALL: [u32; 2] = [0x3D7FE6, 0x0A1838];
/// The macOS window buttons (close, minimise, zoom).
pub const MAC_BUTTONS: [Color32; 3] = [rgb(0xFF5F57), rgb(0xFEBC2E), rgb(0x28C840)];
/// macOS's own System Settings, as the permission guide draws a mock of
/// it: (window, sidebar, chosen row, rows, ink, ink2, hairline) in its dark
/// and light looks, and its blue.
pub const MAC_DARK: [Color32; 7] = [rgb(0x1E1E1E), rgb(0x2A272B), rgb(0x2B2B2B), rgb(0x3A3A3A), rgb(0xECECEC), rgb(0x9A9A9A), rgb(0x48484A)];
pub const MAC_LIGHT: [Color32; 7] = [rgb(0xF6F6F6), rgb(0xE8E6EA), Color32::WHITE, rgb(0xE2E2E2), rgb(0x1D1D1F), rgb(0x86868B), rgb(0xE5E5EA)];
pub const MAC_BLUE: Color32 = rgb(0x0A84FF);
/// The desktop behind the guide's mock windows, and its two wallpapers.
pub const MAC_DESK: Color32 = rgb(0x080A0D);
pub const MAC_WALLS: [[Color32; 2]; 2] = [[rgb(0x2B3A55), rgb(0x6B5B7A)], [rgb(0x24444A), rgb(0x4E5F7A)]];
/// Our own app's row in the mock's list.
pub const MAC_OURS: Color32 = rgb(0x13171C);

/// A colour worked out by code (shading, a model's vertex colours), not
/// one written down.
pub fn rgb_from(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// The same, with a straight (unmultiplied) alpha.
pub fn rgba_from(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Black at `alpha`: shade over pictures (screens, wallpapers), the same in both themes.
pub const fn black(alpha: u8) -> Color32 {
    Color32::from_black_alpha(alpha)
}

/// White at `alpha`: light on pictures (screens, wallpapers), the same in both themes.
pub fn white(alpha: u8) -> Color32 {
    Color32::from_white_alpha(alpha)
}


/// Corners: panels, things inside them, small controls.
pub const R_PANEL: u8 = 20;
pub const R_INNER: u8 = 12;
/// A group of rows on glass (settings).
pub const R_GROUP: u8 = 16;
/// The gap round and between panels.
pub const M: f32 = 12.0;

// ---- Type: Inter for words, JetBrains Mono for every number, unit,
// shortcut, key name and command, Unbounded only in the wordmark. The scale
// is the handoff's (tokens.json `type.scale`).

/// A page's title (22/600).
#[allow(dead_code)] // The new layout (top bar, panels) uses it.
pub const T_PAGE: f32 = 22.0;
/// A section's title, as Keys & mouse's (20/600).
#[allow(dead_code)] // The new layout (top bar, panels) uses it.
pub const T_SECTION: f32 = 20.0;
/// Panel titles and the sheet title (16/600).
pub const T_TITLE: f32 = 16.0;
/// Body, row labels, tabs, buttons (13; buttons 500, the primary 600).
pub const T_BODY: f32 = 13.0;
/// Captions, chips, keys, system names, values (12).
pub const T_META: f32 = 12.0;
/// Mono chips (11).
#[allow(dead_code)] // The new layout (top bar, panels) uses it.
pub const T_CHIP: f32 = 11.0;
/// A big number, as the latency (36/500 mono).
#[allow(dead_code)] // The new layout (top bar, panels) uses it.
pub const T_BIG: f32 = 36.0;
/// The wordmark beside the 24-point symbol (Unbounded Bold 15).
#[allow(dead_code)] // The new layout (top bar, panels) uses it.
pub const T_WORDMARK: f32 = 15.0;
/// Paragraph line height, as a multiple of the size.
pub const LEADING: f32 = 1.45;

/// Inter 600: titles, labels that lead, the primary button.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}
/// Inter 500: buttons, tabs.
#[allow(dead_code)] // The new layout (top bar, panels) uses it.
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}
/// Inter 400: everything else that is words.
pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
/// JetBrains Mono 400: numbers, units, shortcuts, key names, commands.
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}
/// JetBrains Mono 500: the big numbers and the pairing code.
pub fn mono_medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("mono-medium".into()))
}
/// Unbounded Bold: the wordmark, and nothing else.
#[allow(dead_code)] // The new layout (top bar, panels) uses it.
pub fn wordmark(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("wordmark".into()))
}

/// `text`, shortened with an ellipsis to fit `max_w` points.
pub fn fit(painter: &Painter, text: &str, font: &FontId, max_w: f32) -> String {
    let width = |s: &str| painter.layout_no_wrap(s.to_owned(), font.clone(), Color32::PLACEHOLDER).size().x;
    if width(text) <= max_w {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let cut = |n: usize| format!("{}…", chars[..n].iter().collect::<String>().trim_end());
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if width(&cut(mid)) <= max_w { lo = mid } else { hi = mid - 1 }
    }
    // Never end on a combining mark that lost its letter.
    while lo > 0 && is_combining(chars[lo]) {
        lo -= 1;
    }
    cut(lo)
}

fn is_combining(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F | 0x200D | 0xFE0F)
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts(&[]));
    apply_visuals(ctx);
}

/// egui's own widgets (menus, tooltips, text fields, scroll bars) in the
/// active palette.
pub fn apply_visuals(ctx: &egui::Context) {
    let p = pal();
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        *v = if p.theme == Theme::Light { egui::Visuals::light() } else { egui::Visuals::dark() };
        v.panel_fill = p.bg;
        v.window_fill = p.surface;
        v.faint_bg_color = p.surface2;
        v.extreme_bg_color = p.surface;
        v.code_bg_color = p.surface2;
        v.window_stroke = Stroke::new(1.0, p.line2);
        v.window_shadow.color = p.shadow(90);
        v.popup_shadow.color = p.shadow(70);
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(10);
        v.selection.bg_fill = p.accent_soft;
        v.selection.stroke = Stroke::new(1.0, p.accent_text);
        v.hyperlink_color = p.accent_text;
        v.warn_fg_color = p.warn;
        v.error_fg_color = p.danger;
        v.text_cursor.stroke = Stroke::new(2.0, p.accent);
        v.override_text_color = Some(p.ink);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.ink);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.ink2);
        v.widgets.inactive.bg_fill = p.hover;
        v.widgets.inactive.weak_bg_fill = p.surface2;
        v.widgets.hovered.bg_fill = p.hover;
        v.widgets.hovered.weak_bg_fill = p.hover;
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.ink_hover);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.line2);
        v.widgets.active.bg_fill = p.ink3;
        v.widgets.active.weak_bg_fill = p.surface2_hover;
        v.widgets.active.fg_stroke = Stroke::new(1.0, p.ink);
        v.widgets.open.weak_bg_fill = p.surface2;
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        // Thin scroll bars that float over the list, in the panel's greys.
        style.spacing.scroll = egui::style::ScrollStyle::floating();
        style.spacing.scroll.bar_width = 6.0;
        // A list with more below shows its handle even at rest.
        style.spacing.scroll.dormant_handle_opacity = 0.6;
    });
}

fn fonts(extra: &[(String, Vec<u8>)]) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        ("inter", &include_bytes!("../fonts/Inter-Regular.ttf")[..]),
        ("inter-medium", &include_bytes!("../fonts/Inter-Medium.ttf")[..]),
        ("inter-semibold", &include_bytes!("../fonts/Inter-SemiBold.ttf")[..]),
        ("jetbrains-mono", &include_bytes!("../fonts/JetBrainsMono-Regular.ttf")[..]),
        ("jetbrains-mono-medium", &include_bytes!("../fonts/JetBrainsMono-Medium.ttf")[..]),
        ("unbounded-bold", &include_bytes!("../fonts/Unbounded-Bold.ttf")[..]),
    ] {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    // Inter first; egui's own fonts stay behind it for symbols, then the
    // system's fonts for scripts Inter doesn't cover.
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "inter".into());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "jetbrains-mono".into());
    // The ⌘ in shortcut hints: macOS's symbol font draws it best.
    if let Some(bytes) = mac_symbols() {
        // Apple Symbols draws ⌘ smaller and higher than the figures beside
        // it (measured on a Mac with the earlier type: 12 px next to 16 px,
        // its foot 2.5 pt up; 0.2 put it 1.5 pt below the baseline).
        let tweak = egui::FontTweak { scale: 1.25, y_offset_factor: 0.125, ..Default::default() };
        fonts.font_data.insert("mac-symbols".into(), Arc::new(FontData::from_static(bytes).tweak(tweak)));
        fonts.families.entry(FontFamily::Proportional).or_default().push("mac-symbols".into());
        fonts.families.entry(FontFamily::Monospace).or_default().push("mac-symbols".into());
    }
    for (name, bytes) in extra {
        fonts.font_data.insert(name.clone(), Arc::new(FontData::from_owned(bytes.clone())));
        fonts.families.entry(FontFamily::Proportional).or_default().push(name.clone());
    }
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    let mono_fallback = fonts.families[&FontFamily::Monospace].clone();
    fonts.families.insert(FontFamily::Name("semibold".into()), [vec!["inter-semibold".into()], fallback.clone()].concat());
    fonts.families.insert(FontFamily::Name("medium".into()), [vec!["inter-medium".into()], fallback.clone()].concat());
    fonts.families.insert(FontFamily::Name("mono-medium".into()), [vec!["jetbrains-mono-medium".into()], mono_fallback].concat());
    fonts.families.insert(FontFamily::Name("wordmark".into()), [vec!["unbounded-bold".into()], fallback].concat());
    fonts
}

/// macOS's own symbol font, read once (the window's fonts are rebuilt when
/// a name needs another script).
fn mac_symbols() -> Option<&'static [u8]> {
    static BYTES: std::sync::OnceLock<Option<&'static [u8]>> = std::sync::OnceLock::new();
    *BYTES.get_or_init(|| {
        if !cfg!(target_os = "macos") { return None; }
        std::fs::read("/System/Library/Fonts/Apple Symbols.ttf").ok().map(|bytes| &*Box::leak(bytes.into_boxed_slice()))
    })
}

/// Names can be in any script. When one has letters no loaded font
/// covers, find a system font that does and add it behind Inter. Fonts
/// are only read when such a name shows up, so most people never pay.
#[derive(Default)]
pub struct Fallbacks {
    loaded: Vec<(String, Vec<u8>)>,
    tried: std::collections::HashSet<char>,
}

impl Fallbacks {
    pub fn cover<'a>(&mut self, ctx: &egui::Context, names: impl Iterator<Item = &'a str>) {
        let missing: Vec<char> = names
            .flat_map(str::chars)
            .filter(|c| !c.is_control() && !self.tried.contains(c))
            .filter(|c| !ctx.fonts_mut(|f| f.has_glyph(&regular(T_BODY), *c)))
            .collect();
        let mut added = false;
        for c in missing {
            if !self.tried.insert(c) {
                continue;
            }
            if let Some(path) = system_font_for(c)
                && !self.loaded.iter().any(|(n, _)| n == &path)
                && let Ok(bytes) = std::fs::read(&path)
            {
                self.loaded.push((path, bytes));
                added = true;
            }
        }
        if added {
            ctx.set_fonts(fonts(&self.loaded));
        }
    }
}

fn system_font_for(c: char) -> Option<String> {
    if cfg!(target_os = "linux") {
        let out = std::process::Command::new("fc-match").args(["-f", "%{file}", &format!(":charset={:x}", c as u32)]).output().ok()?;
        let path = String::from_utf8(out.stdout).ok()?;
        // Collections (.ttc) hold several faces; egui reads the first.
        return (out.status.success() && !path.is_empty()).then_some(path);
    }
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &["/System/Library/Fonts/Supplemental/Arial Unicode.ttf", "/System/Library/Fonts/PingFang.ttc", "/System/Library/Fonts/AppleSDGothicNeo.ttc"]
    } else {
        &["C:\\Windows\\Fonts\\seguisym.ttf", "C:\\Windows\\Fonts\\msyh.ttc", "C:\\Windows\\Fonts\\malgun.ttf", "C:\\Windows\\Fonts\\Nirmala.ttc"]
    };
    candidates.iter().find(|p| std::path::Path::new(p).exists()).map(|p| (*p).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast(first: Color32, second: Color32) -> f64 {
        let luminance = |color: Color32| {
            let linear = |channel: u8| {
                let value = f64::from(channel) / 255.0;
                if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
        };
        let (a, b) = (luminance(first), luminance(second));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// The tokens exactly as the handoff's `tokens.json` has them.
    #[test]
    fn palettes_carry_the_design_tokens() {
        let (l, d) = (Palette::light(), Palette::dark());
        for (p, tokens) in [
            (l, [("bg", 0xF4F3F0), ("surface", 0xFFFFFF), ("surface2", 0xEFEEEA), ("ink", 0x16171A), ("ink2", 0x6B6D73), ("ink3", 0x8E9096), ("accent", 0x2F54EB), ("accent_soft", 0xE9EDFF), ("accent_text", 0x2443C9), ("ok", 0x1F9D63), ("warn", 0xC27A0E), ("warn_soft", 0xFBF2E2), ("danger", 0xC0392B), ("seg_on", 0xFFFFFF), ("switch_off", 0xD9D7D1), ("focus", 0x2F54EB)]),
            (d, [("bg", 0x111214), ("surface", 0x1A1B1F), ("surface2", 0x25262B), ("ink", 0xEDEDEB), ("ink2", 0x9A9CA3), ("ink3", 0x75777E), ("accent", 0x4D6DFF), ("accent_soft", 0x1D2445), ("accent_text", 0x8EA4FF), ("ok", 0x3CC985), ("warn", 0xE9A23B), ("warn_soft", 0x2A2216), ("danger", 0xF0675A), ("seg_on", 0x34353B), ("switch_off", 0x3A3B41), ("focus", 0x7E97FF)]),
        ] {
            let colours = p.colours();
            for (name, hex) in tokens {
                let got = colours.iter().find(|(n, _)| *n == name).map(|(_, c)| *c);
                assert_eq!(got, Some(rgb(hex)), "{:?} {name}", p.theme);
            }
        }
        assert_eq!(l.line, Color32::from_rgba_unmultiplied(22, 23, 26, 20));
        assert_eq!(d.line2, Color32::from_rgba_unmultiplied(255, 255, 255, 31));
        assert_eq!(l.floor, Color32::from_rgba_unmultiplied(22, 23, 26, 33));
        assert_eq!(d.floor, Color32::from_black_alpha(140));
    }

    /// Both themes are complete: every colour set, the opaque ones opaque,
    /// the see-through ones see-through, and the two themes apart.
    #[test]
    fn both_palettes_are_complete() {
        let (l, d) = (Palette::light(), Palette::dark());
        let translucent = ["line", "line2", "floor", "grid", "hover", "well", "field_edge_lit", "glass_top", "glass", "room_tint", "glass_edge", "ring"];
        for p in [l, d] {
            for (name, c) in p.colours() {
                assert_ne!(c, Color32::PLACEHOLDER, "{name}");
                if translucent.contains(&name) {
                    assert!(c.a() > 0 && c.a() < 255, "{:?} {name} is see-through", p.theme);
                } else {
                    assert_eq!(c.a(), 255, "{:?} {name} is opaque", p.theme);
                }
            }
        }
        for name in ["bg", "surface", "surface2", "ink", "ink2", "ink3", "accent", "accent_soft"] {
            let get = |p: &Palette| p.colours().into_iter().find(|(n, _)| *n == name).unwrap().1;
            assert_ne!(get(&l), get(&d), "{name} differs between the themes");
        }
    }

    /// Text: ink and ink2 at 4.5:1 on every surface they are drawn on, in
    /// both themes; the accent's text on its soft fill too.
    #[test]
    fn text_meets_four_and_a_half_to_one_in_both_themes() {
        for p in [Palette::light(), Palette::dark()] {
            let cases = [
                ("ink on surface", p.ink, p.surface),
                ("ink2 on surface", p.ink2, p.surface),
                ("ink on bg", p.ink, p.bg),
                ("ink2 on bg", p.ink2, p.bg),
                ("ink on surface2", p.ink, p.surface2),
                ("ink on the chosen segment", p.ink, p.seg_on),
                ("ink on a hovered quiet button", p.ink, p.surface2_hover),
                ("accent text on accent soft", p.accent_text, p.accent_soft),

                ("ink on the sidebar", p.ink, p.sidebar),
                ("ink2 on the sidebar", p.ink2, p.sidebar),
                ("ink on the warning fill", p.ink, p.warn_soft),
            ];
            for (what, fg, bg) in cases {
                let c = contrast(fg, bg);
                assert!(c >= 4.5, "{:?}: {what}: {c:.2}:1", p.theme);
            }
            // White on the accent: 4.5:1 in light; the handoff's dark accent
            // (#4D6DFF) gives white 4.24:1, so the dark primary button's
            // label is held to 4.2 (it is semibold).
            for (what, bg) in [("on-accent on accent", p.accent), ("on-accent on hovered accent", p.accent_hover), ("on-accent on pressed accent", p.accent_press)] {
                let c = contrast(p.on_accent, bg);
                assert!(c >= if p.theme == Theme::Light { 4.5 } else { 4.2 }, "{:?}: {what}: {c:.2}:1", p.theme);
            }
            // Status marks, the focus ring and the accent itself: 3:1 (they
            // are shapes, not running text).
            for (what, fg, bg) in [("ok", p.ok, p.surface), ("warn", p.warn, p.surface), ("danger", p.danger, p.surface), ("accent", p.accent, p.surface), ("focus", p.focus, p.bg), ("hints", p.ink3, p.surface)] {
                let c = contrast(fg, bg);
                assert!(c >= 3.0, "{:?}: {what}: {c:.2}:1", p.theme);
            }
        }
    }

    #[test]
    fn a_cross_fade_starts_and_ends_on_each_palette() {
        let (l, d) = (Palette::light(), Palette::dark());
        assert_eq!(Palette::lerp(&d, &l, 0.0), d);
        assert_eq!(Palette::lerp(&d, &l, 1.0), l);
        let half = Palette::lerp(&d, &l, 0.5);
        assert!(half.bg.r() > d.bg.r() && half.bg.r() < l.bg.r());
    }

    #[test]
    fn appearance_reads_what_settings_write() {
        for a in Appearance::ALL {
            assert_eq!(Appearance::parse(a.key()), a);
        }
        assert_eq!(Appearance::parse("nonsense"), Appearance::System);
        assert_eq!(Appearance::System.theme(Some(Theme::Light)), Theme::Light);
        assert_eq!(Appearance::System.theme(None), Theme::Dark);
        assert_eq!(Appearance::Dark.theme(Some(Theme::Light)), Theme::Dark);
    }

    /// No colour is written down outside this file: every other file of
    /// the window takes its colours from the palette or the fixed colours here.
    #[test]
    fn no_colour_literals_outside_style() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![];
        let mut dirs = vec![root.clone()];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() { dirs.push(path) } else if path.extension().is_some_and(|e| e == "rs") { files.push(path) }
            }
        }
        let banned = ["Color32::from_", "from_black_alpha(", "from_white_alpha(", "from_gray(", "Color32::WHITE", "Color32::BLACK", "Color32::GRAY", "Color32::RED", "Color32::GREEN", "Color32::BLUE", "Color32::YELLOW", "Color32::LIGHT", "Color32::DARK"];
        let mut found = vec![];
        // Only the window's painting code: the daemon link's numbers aren't colours.
        for path in files.iter().filter(|p| !p.ends_with("style.rs") && !p.ends_with("daemon.rs")) {
            let text = std::fs::read_to_string(path).unwrap();
            // Tests may build what they compare against.
            let code = text.split("#[cfg(test)]\nmod tests").next().unwrap_or(&text);
            for (n, line) in code.lines().enumerate() {
                let hexes = line.match_indices("0x").filter(|(i, _)| line[i + 2..].chars().take_while(char::is_ascii_hexdigit).count() == 6).count();
                let bytes = line.contains("[0x") && line.matches(", 0x").count() >= 2;
                if banned.iter().any(|b| line.contains(b)) || hexes > 0 || bytes {
                    found.push(format!("{}:{}: {}", path.strip_prefix(&root).unwrap().display(), n + 1, line.trim()));
                }
            }
        }
        assert!(found.is_empty(), "colour literals outside style.rs:\n{}", found.join("\n"));
    }

    /// Every face is built in and is its own: Inter's three weights,
    /// JetBrains Mono's two (every figure as wide as the next), Unbounded.
    #[test]
    fn the_type_faces_are_built_in() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| {
            let p = ctx.layer_painter(egui::LayerId::background());
            let w = |text: &str, font: FontId| p.layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER).size().x;
            // (JetBrains Mono's two weights share their advance widths.)
            assert!(w("oneMK", mono_medium(T_BODY)) > 20.0);
            let fonts = [regular(T_BODY), medium(T_BODY), semibold(T_BODY), mono(T_BODY), wordmark(T_BODY)];
            let widths: Vec<f32> = fonts.iter().map(|f| w("oneMK Desk", f.clone())).collect();
            for (i, a) in widths.iter().enumerate() {
                assert!(*a > 20.0, "{:?}", fonts[i]);
                for b in &widths[i + 1..] {
                    assert!((a - b).abs() > 0.05, "two faces measure the same: {widths:?}");
                }
            }
            assert!((w("1111", mono(T_BODY)) - w("8888", mono(T_BODY))).abs() < 0.01, "mono figures are even");
            for c in ['⌘', '·', '…', '→', '✓'] {
                assert!(ctx.fonts_mut(|f| f.has_glyph(&regular(T_BODY), c) || f.has_glyph(&mono(T_BODY), c)), "{c}");
            }
        });
        out.textures_delta.clear();
    }

    #[test]
    fn long_text_is_shortened_to_fit() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| {
            let p = ctx.layer_painter(egui::LayerId::background());
            let font = regular(T_BODY);
            assert_eq!(fit(&p, "desk", &font, 200.0), "desk");
            let long = "A very long computer name that will not fit anywhere at all";
            let short = fit(&p, long, &font, 120.0);
            assert!(short.ends_with('…') && short.len() < long.len(), "{short}");
            assert!(p.layout_no_wrap(short, font.clone(), pal().ink).size().x <= 120.0);
            assert_eq!(fit(&p, long, &font, 1.0), "…");
        });
        out.textures_delta.clear();
    }
}
