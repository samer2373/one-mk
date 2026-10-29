//! The drawn pointer on X11 (see `cursoroverlay.rs`).
//!
//! The X server always draws its pointer, mouse or no mouse (its virtual
//! core pointer exists either way, and XTest moves it), so `auto` never
//! draws here; the probe says so without asking the server. `always`
//! draws anyway: an override-redirect window (no window manager frame or
//! focus) shaped like the arrow, with an empty input shape so every click
//! goes through to what is under it. Its look is a background pixmap, so
//! the server repaints it without us: no thread, no event loop. Moving it
//! is one `ConfigureWindow` request per injected move.

use crate::cursoroverlay::{Overlay, Platform, Probe, Px, arrow, arrow_scale};
use anyhow::{Context, Result};
use one_mk_proto::Pos;
use one_mk_proto::overlay::OsCursor;
use x11rb::connection::Connection;
use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
use x11rb::protocol::xproto::{
    ChangeGCAux, ChangeWindowAttributesAux, ClipOrdering, ConfigureWindowAux, ConnectionExt as _, CreateGCAux, CreateWindowAux, Rectangle, StackMode,
    WindowClass,
};
use x11rb::rust_connection::RustConnection;

pub(crate) struct X11 {
    /// `None`: `$DISPLAY`.
    display: Option<String>,
}

impl X11 {
    pub(crate) fn from_env() -> Self {
        X11 { display: None }
    }

    #[cfg(test)]
    fn on(display: &str) -> Self {
        X11 { display: Some(display.to_owned()) }
    }
}

impl Platform for X11 {
    fn probe(&mut self) -> Probe {
        Probe { pointing_device: None, os_cursor: OsCursor::Shown }
    }

    fn create(&mut self, at: Pos) -> Result<Box<dyn Overlay>> {
        Ok(Box::new(Window::open(self.display.as_deref(), at)?))
    }
}

struct Window {
    conn: RustConnection,
    win: u32,
}

/// Rectangles (one per run of pixels in a row) covering the pixels for
/// which `keep` holds.
fn runs(rows: &[Vec<Px>], keep: impl Fn(Px) -> bool) -> Vec<Rectangle> {
    let mut out = Vec::new();
    for (y, row) in rows.iter().enumerate() {
        let mut x = 0;
        while x < row.len() {
            if !keep(row[x]) {
                x += 1;
                continue;
            }
            let start = x;
            while x < row.len() && keep(row[x]) {
                x += 1;
            }
            out.push(Rectangle { x: start as i16, y: y as i16, width: (x - start) as u16, height: 1 });
        }
    }
    out
}

impl Window {
    fn open(display: Option<&str>, at: Pos) -> Result<Window> {
        let (conn, screen) = x11rb::connect(display).context("can't open the X display")?;
        conn.shape_query_version()?.reply().context("the X server has no SHAPE extension")?;
        let s = &conn.setup().roots[screen];
        let (root, depth, white, black) = (s.root, s.root_depth, s.white_pixel, s.black_pixel);
        // X has no logical pixels; draw bigger on a dense screen.
        let dpi = if s.width_in_millimeters > 0 {
            f64::from(s.width_in_pixels) * 25.4 / f64::from(s.width_in_millimeters)
        } else {
            96.0
        };
        let rows = arrow(arrow_scale(dpi / 96.0));
        let (w, h) = (rows[0].len() as u16, rows.len() as u16);
        let (x, y) = coords(at);

        let win = conn.generate_id()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            root,
            x,
            y,
            w,
            h,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().override_redirect(1).save_under(1),
        )?
        .check()
        .context("can't create the pointer window")?;
        let made = (|| -> Result<()> {
            // The look: white fill, black outline, in a pixmap the server
            // paints the window from.
            let pixmap = conn.generate_id()?;
            conn.create_pixmap(depth, pixmap, win, w, h)?;
            let gc = conn.generate_id()?;
            conn.create_gc(gc, pixmap, &CreateGCAux::new().foreground(white))?;
            conn.poly_fill_rectangle(pixmap, gc, &[Rectangle { x: 0, y: 0, width: w, height: h }])?;
            conn.change_gc(gc, &ChangeGCAux::new().foreground(black))?;
            conn.poly_fill_rectangle(pixmap, gc, &runs(&rows, |p| p == Px::Outline))?;
            conn.change_window_attributes(win, &ChangeWindowAttributesAux::new().background_pixmap(pixmap))?;
            conn.free_gc(gc)?;
            conn.free_pixmap(pixmap)?;
            // Arrow-shaped, and not there at all for input.
            let shape = runs(&rows, |p| p != Px::Clear);
            conn.shape_rectangles(SO::SET, SK::BOUNDING, ClipOrdering::UNSORTED, win, 0, 0, &shape)?;
            conn.shape_rectangles(SO::SET, SK::INPUT, ClipOrdering::UNSORTED, win, 0, 0, &[])?;
            conn.map_window(win)?.check()?;
            Ok(())
        })();
        if let Err(e) = made {
            let _ = conn.destroy_window(win);
            let _ = conn.flush();
            return Err(e.context("can't set up the pointer window"));
        }
        Ok(Window { conn, win })
    }
}

fn coords(p: Pos) -> (i16, i16) {
    let c = |v: f64| v.round().clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
    (c(p.x), c(p.y))
}

impl Overlay for Window {
    fn move_to(&mut self, pos: Pos) {
        let (x, y) = coords(pos);
        // Raised each time: a newly mapped window would cover it otherwise.
        let aux = ConfigureWindowAux::new().x(i32::from(x)).y(i32::from(y)).stack_mode(StackMode::ABOVE);
        let _ = self.conn.configure_window(self.win, &aux);
        let _ = self.conn.flush();
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        let _ = self.conn.destroy_window(self.win);
        let _ = self.conn.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Inject;
    use crate::cursoroverlay::{DrawMode, InjectTx};
    use std::io::BufRead;
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;
    use x11rb::protocol::xproto::MapState;

    /// A private X server, killed when dropped.
    struct Xvfb {
        child: Child,
        display: String,
    }

    impl Xvfb {
        fn start() -> Option<Xvfb> {
            let child = Command::new("Xvfb")
                .args(["-displayfd", "1", "-screen", "0", "1024x768x24", "-nolisten", "tcp", "-noreset"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            let Ok(mut child) = child else {
                eprintln!("Xvfb not installed: skipping");
                return None;
            };
            let mut line = String::new();
            std::io::BufReader::new(child.stdout.take().expect("piped")).read_line(&mut line).unwrap();
            assert!(line.trim().parse::<u32>().is_ok(), "Xvfb did not start: {line:?}");
            Some(Xvfb { child, display: format!(":{}", line.trim()) })
        }
    }

    impl Drop for Xvfb {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// The override-redirect windows mapped on the root, with their place.
    fn overlays(conn: &RustConnection, root: u32) -> Vec<(u32, i16, i16, u16, u16)> {
        let tree = conn.query_tree(root).unwrap().reply().unwrap();
        tree.children
            .into_iter()
            .filter_map(|w| {
                let a = conn.get_window_attributes(w).ok()?.reply().ok()?;
                (a.override_redirect && a.map_state == MapState::VIEWABLE).then_some(w)
            })
            .map(|w| {
                let g = conn.get_geometry(w).unwrap().reply().unwrap();
                (w, g.x, g.y, g.width, g.height)
            })
            .collect()
    }

    /// Polls for up to a second (the overlay talks on its own connection).
    fn eventually<T>(mut f: impl FnMut() -> Option<T>) -> T {
        for _ in 0..100 {
            if let Some(v) = f() {
                return v;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out");
    }

    #[test]
    fn always_draws_a_click_through_arrow_on_xvfb() {
        let Some(server) = Xvfb::start() else { return };
        let (conn, screen) = x11rb::connect(Some(&server.display)).unwrap();
        let root = conn.setup().roots[screen].root;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // auto: X draws its own pointer; nothing is made.
        let inject = InjectTx::new(tx.clone(), Some(Box::new(X11::on(&server.display))), DrawMode::Auto);
        inject.send(Inject::Controlled(true)).unwrap();
        inject.send(Inject::MoveTo(Pos::new(100.0, 200.0))).unwrap();
        assert!(!inject.overlay_active());
        assert!(overlays(&conn, root).is_empty());

        inject.set_overlay_mode(DrawMode::Always);
        assert!(inject.overlay_active());
        let (win, x, y, w, h) = eventually(|| overlays(&conn, root).pop());
        assert_eq!((x, y, w, h), (100, 200, 12, 19));
        // No input shape at all: clicks go through.
        let input = conn.shape_get_rectangles(win, SK::INPUT).unwrap().reply().unwrap();
        assert!(input.rectangles.is_empty(), "{:?}", input.rectangles);
        // Arrow-shaped: the tip is in, the pixel right of it is out.
        let bounding = conn.shape_get_rectangles(win, SK::BOUNDING).unwrap().reply().unwrap();
        let inside = |px: i16, py: i16| {
            bounding.rectangles.iter().any(|r| px >= r.x && px < r.x + r.width as i16 && py >= r.y && py < r.y + r.height as i16)
        };
        assert!(inside(0, 0) && !inside(1, 0) && inside(5, 10));

        inject.send(Inject::MoveTo(Pos::new(300.5, 400.0))).unwrap();
        eventually(|| (overlays(&conn, root) == [(win, 301, 400, 12, 19)]).then_some(()));

        // Control ends: the window goes. Many cycles leave nothing behind.
        inject.send(Inject::Controlled(false)).unwrap();
        eventually(|| overlays(&conn, root).is_empty().then_some(()));
        for i in 0..20 {
            inject.send(Inject::Controlled(true)).unwrap();
            inject.send(Inject::MoveTo(Pos::new(f64::from(i), 5.0))).unwrap();
            inject.send(Inject::Controlled(false)).unwrap();
        }
        eventually(|| conn.query_tree(root).unwrap().reply().unwrap().children.is_empty().then_some(()));
    }
}
