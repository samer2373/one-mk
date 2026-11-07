//! The window itself. On macOS there is no grey title bar: the close,
//! minimise and zoom buttons sit inside the glass sidebar, and the window
//! is see-through, with the desktop behind it blurred by the window server
//! (as Terminal and the system's own sidebars do). The glass is then real
//! glass: the wallpaper's colours come through every panel. Elsewhere the
//! window is opaque and the panels blur the desk drawn behind them
//! (glass.rs).
//!
//! The desktop's "Reduce transparency" setting (and `ONE_MK_NO_GLASS=1`)
//! turns all of it off: solid panels on a solid window.

/// The window shows the desktop behind it, blurred. Not in the Night
/// design (solid surfaces, glass only where things float), so only the
/// debug preview turns it on.
pub fn see_through() -> bool {
    fake::desktop() && !crate::glass::refused() && !blur_failed()
}

/// The desktop asks apps to draw solid surfaces (macOS: Accessibility >
/// Display > Reduce transparency). Read once, when the window opens.
pub fn reduce_transparency() -> bool {
    #[cfg(target_os = "macos")]
    {
        static ASKED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ASKED.get_or_init(mac::reduce_transparency)
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// Where the macOS window buttons go, in points from the window's top left:
/// centred in this strip, the close button this far in, inside the
/// sidebar's top corner beside the logo.
pub const BUTTONS_Y: egui::Rangef = egui::Rangef { min: 0.0, max: 44.0 };
pub const BUTTONS_X: f32 = 18.0;
/// The buttons' width, margins included: the logo starts after them.
pub const BUTTONS_W: f32 = 76.0;

/// The window buttons sit inside the sidebar (macOS).
pub fn mac_frame() -> bool {
    cfg!(target_os = "macos") || fake::desktop()
}

/// The room behind everything: a dark ground lit by two faint lights, a
/// cool one high on the right and the desk lamp's warm one low down, so
/// the glass over it has colour to frost. See-through, the ground is a
/// dark tint over the blurred desktop instead, dark enough that grey text
/// on it keeps 4.5:1 over a white wallpaper.
pub fn paint_room(p: &egui::Painter, full: egui::Rect) {
    use crate::style::{LAMP, ROOM, ROOM_TINT};
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    fake::paint_desktop(p, full);
    p.rect_filled(full, 0.0, if see_through() { ROOM_TINT } else { ROOM });
    // A quiet room: no lights of its own; the computers carry the colour.
    let _ = LAMP;
}

/// A soft round light: full colour in the middle, gone at `radius`, eased
/// over three rings so it has no visible edge.
pub fn light(p: &egui::Painter, center: egui::Pos2, radius: f32, color: egui::Color32) {
    use egui::epaint::{Mesh, Vertex, WHITE_UV};
    const SIDES: u32 = 48;
    const RINGS: [(f32, f32); 4] = [(0.0, 1.0), (0.35, 0.62), (0.7, 0.2), (1.0, 0.0)];
    let mut m = Mesh::default();
    for (k, (r, a)) in RINGS.iter().enumerate() {
        let n = if k == 0 { 1 } else { SIDES };
        for i in 0..n {
            let t = i as f32 / SIDES as f32 * std::f32::consts::TAU;
            m.vertices.push(Vertex { pos: center + radius * r * egui::vec2(t.cos(), t.sin()), uv: WHITE_UV, color: color.gamma_multiply(*a) });
        }
    }
    for i in 0..SIDES {
        let j = (i + 1) % SIDES;
        m.add_triangle(0, 1 + i, 1 + j);
        for ring in 0..RINGS.len() as u32 - 2 {
            let (a, b) = (1 + ring * SIDES, 1 + (ring + 1) * SIDES);
            m.add_triangle(a + i, b + i, b + j);
            m.add_triangle(a + i, b + j, a + j);
        }
    }
    p.add(m);
}

/// What the window is cleared to before the room is painted.
pub fn clear_color() -> [f32; 4] {
    if see_through() { [0.0; 4] } else { crate::style::GROUND.to_normalized_gamma_f32() }
}

/// The window options that make the frame: on macOS, content under a
/// transparent title bar with no title.
pub fn viewport(b: egui::ViewportBuilder) -> egui::ViewportBuilder {
    if mac_frame() { b.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false) } else { b }
}

/// Every frame: put the window buttons back where the sidebar expects
/// them (AppKit moves them again on each layout).
pub fn place_buttons(ctx: &egui::Context, frame: &eframe::Frame) {
    if mac_frame() {
        frame.set_traffic_lights_position(ctx, BUTTONS_Y, BUTTONS_X);
    }
}

/// Once the window exists: ask the window server to blur what is behind it.
pub fn install(cc: &eframe::CreationContext<'_>) {
    #[cfg(target_os = "macos")]
    if see_through() && !mac::blur_behind(cc, BLUR_RADIUS) {
        eprintln!("one-mk-app: the window can't blur the desktop behind it; drawing it solid");
        SOLID.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = cc;
}

/// The window server's blur radius, in pixels: soft enough that a busy
/// wallpaper reads as colour, not shapes.
#[cfg(target_os = "macos")]
const BLUR_RADIUS: i64 = 60;

#[cfg(target_os = "macos")]
static SOLID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// See-through was asked for but the blur failed: draw the ground solid,
/// so the window never shows a sharp desktop through it.
pub fn blur_failed() -> bool {
    #[cfg(target_os = "macos")]
    return SOLID.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(target_os = "macos"))]
    false
}

#[cfg(target_os = "macos")]
mod mac {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use std::ffi::c_void;

    // Private, but stable for over a decade: Terminal.app uses them, and
    // so does winit's `set_blur`, which eframe doesn't offer.
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGSMainConnectionID() -> *mut c_void;
        fn CGSSetWindowBackgroundBlurRadius(connection: *mut c_void, window: isize, radius: i64) -> i32;
    }

    pub fn reduce_transparency() -> bool {
        // SAFETY: a class method with no arguments returning the shared
        // workspace, and a BOOL getter on it (both since macOS 10.10).
        unsafe {
            let ws: *mut AnyObject = msg_send![objc2::class!(NSWorkspace), sharedWorkspace];
            !ws.is_null() && msg_send![&*ws, accessibilityDisplayShouldReduceTransparency]
        }
    }

    pub fn blur_behind(cc: &eframe::CreationContext<'_>, radius: i64) -> bool {
        let Ok(handle) = cc.window_handle() else { return false };
        let RawWindowHandle::AppKit(h) = handle.as_raw() else { return false };
        // SAFETY: on the main thread (eframe calls the app's creator there);
        // the view outlives this call, and `window` and `windowNumber` take
        // no arguments and return an object pointer and an NSInteger.
        unsafe {
            let view = h.ns_view.as_ptr().cast::<AnyObject>();
            let window: *mut AnyObject = msg_send![&*view, window];
            if window.is_null() {
                return false;
            }
            let number: isize = msg_send![&*window, windowNumber];
            CGSSetWindowBackgroundBlurRadius(CGSMainConnectionID(), number, radius) == 0
        }
    }
}

/// Development aid, debug builds only: `ONE_MK_APP_FAKE_DESKTOP=file.ppm`
/// draws that picture (already blurred, as the window server would) behind
/// a see-through window with the macOS frame and its window buttons, so
/// the Mac look can be seen and screenshotted on any system.
mod fake {
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    static PICTURE: std::sync::OnceLock<Option<egui::ColorImage>> = std::sync::OnceLock::new();

    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    fn picture() -> &'static Option<egui::ColorImage> {
        PICTURE.get_or_init(|| {
            let bytes = std::fs::read(std::env::var_os("ONE_MK_APP_FAKE_DESKTOP")?).ok()?;
            // Binary PPM: "P6 <w> <h> 255" then RGB.
            let mut fields = Vec::new();
            let mut at = 0;
            while fields.len() < 4 {
                while bytes.get(at)?.is_ascii_whitespace() {
                    at += 1;
                }
                let start = at;
                while !bytes.get(at)?.is_ascii_whitespace() {
                    at += 1;
                }
                fields.push(std::str::from_utf8(&bytes[start..at]).ok()?.to_owned());
            }
            let (w, h): (usize, usize) = (fields[1].parse().ok()?, fields[2].parse().ok()?);
            let rgb = bytes.get(at + 1..at + 1 + w * h * 3)?;
            Some(egui::ColorImage::from_rgb([w, h], rgb))
        })
    }

    pub fn desktop() -> bool {
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        return picture().is_some();
        #[cfg(not(any(debug_assertions, feature = "design-evidence")))]
        false
    }

    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    pub fn paint_desktop(p: &egui::Painter, full: egui::Rect) {
        let Some(img) = picture() else { return };
        let tex = p.ctx().data_mut(|d| d.get_temp::<egui::TextureHandle>(egui::Id::unique("fake-desktop"))).unwrap_or_else(|| {
            let t = p.ctx().load_texture("fake-desktop", img.clone(), egui::TextureOptions::LINEAR);
            p.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("fake-desktop"), t.clone()));
            t
        });
        p.image(tex.id(), full, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }

    /// The three window buttons, drawn where AppKit would put them.
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    pub fn paint_buttons(p: &egui::Painter) {
        if !desktop() {
            return;
        }
        let y = super::BUTTONS_Y.center();
        for (k, c) in [(0xFF, 0x5F, 0x57), (0xFE, 0xBC, 0x2E), (0x28, 0xC8, 0x40)].into_iter().enumerate() {
            p.circle(egui::pos2(super::BUTTONS_X + 6.0 + 20.0 * k as f32, y), 6.0, egui::Color32::from_rgb(c.0, c.1, c.2), egui::Stroke::new(0.5, egui::Color32::from_black_alpha(60)));
        }
    }
}

/// The macOS window buttons in a fake desktop screenshot (debug only).
#[cfg(any(debug_assertions, feature = "design-evidence"))]
pub fn paint_fake_buttons(p: &egui::Painter) {
    fake::paint_buttons(p);
}
