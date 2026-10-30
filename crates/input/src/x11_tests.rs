//! The X11 backend against a private `Xvfb` (skipped where it isn't
//! installed): injection observed from a second X client, capture driven
//! through XTest the way a real mouse would push against an edge, and the
//! grab ending with the process, even on `kill -9`.

use super::*;
use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use crate::Bus;
use x11rb::protocol::xproto::GrabStatus;

const W: i16 = 1280;
const H: i16 = 800;

#[test]
#[ignore = "explicit private-Xvfb remapping safety regression"]
fn temporary_remap_restoration_races_a_delayed_client() {
    for count in [1, 32] {
        delayed_remap_client(count, MappingLifetime::Immediate);
    }
}

#[test]
#[ignore = "explicit private-Xvfb retained-pool delivery experiment"]
fn retained_pool_delivers_only_before_retirement() {
    delayed_remap_client(200, MappingLifetime::UntilTranslated);
    delayed_remap_client(200, MappingLifetime::QuietDeadline);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MappingLifetime {
    Immediate,
    UntilTranslated,
    QuietDeadline,
}

fn delayed_remap_client(repetitions: usize, lifetime: MappingLifetime) {
    let server = Xvfb::start().expect("this acceptance experiment requires Xvfb");
    let injector = server.observer();
    let reader = server.observer();
    let setup = injector.conn.setup();
    let first = setup.min_keycode;
    let count = setup.max_keycode - first + 1;
    let before = injector.conn.get_keyboard_mapping(first, count).unwrap().reply().unwrap();
    let width = usize::from(before.keysyms_per_keycode);
    let modifiers = injector.conn.get_modifier_mapping().unwrap().reply().unwrap();
    let down = injector.conn.query_keymap().unwrap().reply().unwrap();
    let offsets: Vec<_> = before.keysyms.chunks_exact(width).enumerate().rev().filter_map(|(index, symbols)| {
        let keycode = first + u8::try_from(index).unwrap();
        (symbols.iter().all(|symbol| *symbol == 0)
            && !modifiers.keycodes.contains(&keycode)
            && down.keys[usize::from(keycode / 8)] & (1 << (keycode % 8)) == 0)
            .then_some(index)
    }).take(if lifetime == MappingLifetime::Immediate { 1 } else { 4 }).collect();
    assert_eq!(offsets.len(), if lifetime == MappingLifetime::Immediate { 1 } else { 4 }, "unused, unheld, non-modifier keycodes");
    let keycodes: Vec<_> = offsets.iter().map(|offset| first + u8::try_from(*offset).unwrap()).collect();
    let observed_keycodes = keycodes.clone();
    let mut xev = TestXev(Command::new("stdbuf").args(["-oL", "xev", "-display", &server.display, "-root", "-event", "keyboard"])
        .stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("xev and stdbuf are required"));
    eventually("Xlib observer ready", || reader.conn.get_window_attributes(reader.root).unwrap().reply().unwrap()
        .all_event_masks.contains(xproto::EventMask::KEY_PRESS));
    let xev_pid = i32::try_from(xev.0.id()).unwrap();
    assert_eq!(unsafe { libc::kill(xev_pid, libc::SIGSTOP) }, 0);
    let mut stopped = 0;
    assert_eq!(unsafe { libc::waitpid(xev_pid, &mut stopped, libc::WUNTRACED) }, xev_pid);
    assert!(libc::WIFSTOPPED(stopped));
    let output = xev.0.stdout.take().unwrap();
    let (lines, translated) = std::sync::mpsc::channel();
    let xlib_reader = std::thread::spawn(move || {
        let mut pressed = false;
        for line in std::io::BufReader::new(output).lines() {
            let Ok(line) = line else { break };
            if line.starts_with("KeyPress event,") { pressed = true; }
            if line.starts_with("KeyRelease event,") { pressed = false; }
            if observed_keycodes.iter().any(|keycode| line.contains(&format!("keycode {keycode} (keysym"))) && lines.send((pressed, line)).is_err() {
                break;
            }
        }
    });
    reader.conn.change_window_attributes(reader.root, &xproto::ChangeWindowAttributesAux::new()
        .event_mask(xproto::EventMask::KEY_PRESS | xproto::EventMask::KEY_RELEASE)).unwrap().check().unwrap();
    reader.conn.set_input_focus(xproto::InputFocus::POINTER_ROOT, reader.root, CURRENT_TIME).unwrap().check().unwrap();
    let mut keyboard = crate::X11Keyboard::connect(&server.display).unwrap();
    let dump = || {
        let output = Command::new("xkbcomp").args(["-xkb", &server.display, "-"]).output().expect("xkbcomp is required");
        assert!(output.status.success(), "xkbcomp: {}", String::from_utf8_lossy(&output.stderr));
        output.stdout
    };
    let native_before = dump();
    let held = 38;
    injector.fake(xproto::KEY_PRESS_EVENT, held, injector.root, 0, 0);
    let mut expected = Vec::new();
    let mut retained = Vec::new();
    for (index, wanted) in ['ı', 'ł', '@', 'z'].into_iter().cycle().take(repetitions).enumerate() {
        let slot = if lifetime == MappingLifetime::Immediate { 0 } else { index % 4 };
        let offset = offsets[slot];
        let keycode = keycodes[slot];
        let symbols = vec![0x0100_0000 | u32::from(wanted); width];
        let mut restore = None;
        if lifetime == MappingLifetime::Immediate || index < 4 {
            let guard = TestMapping { conn: &injector.conn, keycode,
                original: &before.keysyms[offset * width..(offset + 1) * width], active: true };
            injector.conn.change_keyboard_mapping(1, keycode, before.keysyms_per_keycode, &symbols).unwrap().check().unwrap();
            if lifetime == MappingLifetime::Immediate { restore = Some(guard); }
            else { retained.push(guard); }
        }
        let during = keyboard.snapshot().unwrap();
        assert_eq!(during.resolver.resolve(u32::from(keycode) - 8, during.masks), Some(wanted.to_string()));
        injector.fake(xproto::KEY_PRESS_EVENT, keycode, injector.root, 0, 0);
        injector.fake(xproto::KEY_RELEASE_EVENT, keycode, injector.root, 0, 0);
        if let Some(mut restore) = restore { restore.restore().unwrap(); }
        expected.push(u32::from(wanted));
    }
    if lifetime == MappingLifetime::QuietDeadline {
        std::thread::sleep(Duration::from_millis(1600));
        for mapping in &mut retained { mapping.restore().unwrap(); }
    }
    for keycode in &keycodes { assert!(!injector.key_down(*keycode)); }
    assert!(injector.key_down(held), "a different client's held key must not be disturbed");
    injector.fake(xproto::KEY_RELEASE_EVENT, held, injector.root, 0, 0);
    let mut presses = 0;
    let mut notifications = 0;
    let mut symbols = Vec::new();
    while let Some(event) = reader.conn.poll_for_event().unwrap() {
        match event {
            x11rb::protocol::Event::MappingNotify(_) => notifications += 1,
            x11rb::protocol::Event::KeyPress(event) if keycodes.contains(&event.detail) => {
                presses += 1;
                let snapshot = keyboard.snapshot().unwrap();
                if let Some(text) = snapshot.resolver.resolve(u32::from(event.detail) - 8, snapshot.masks) {
                    symbols.extend(text.chars().map(u32::from));
                }
            }
            _ => {}
        }
    }
    assert_eq!(presses, repetitions, "the reader must receive every actual XTest press");
    assert!(notifications > 0, "the reader must receive MappingNotify");
    assert_eq!(unsafe { libc::kill(xev_pid, libc::SIGCONT) }, 0);
    let mut xlib_presses = 0;
    let mut awaiting_release = None;
    while xlib_presses < repetitions || awaiting_release.is_some() {
        let (pressed, line) = translated.recv_timeout(Duration::from_secs(5)).expect("Xlib must translate every press and release");
        if !pressed && awaiting_release.is_none() { continue; }
        let index = if pressed { xlib_presses } else { xlib_presses - 1 };
        let keycode = keycodes[if lifetime == MappingLifetime::Immediate { 0 } else { index % 4 }];
        assert!(line.contains(&format!("keycode {keycode} (keysym")), "Xlib key order: {line}");
        if pressed {
            assert!(awaiting_release.is_none(), "Xlib must release before the next press");
            awaiting_release = Some(keycode);
            xlib_presses += 1;
        } else {
            assert_eq!(awaiting_release.take(), Some(keycode));
        }
        if lifetime == MappingLifetime::UntilTranslated {
            let symbol = 0x0100_0000 | expected[index];
            assert!(line.contains(&format!("(keysym {symbol:#x},")), "Xlib event={index} expected={symbol:#x}: {line}");
        } else {
            assert!(line.contains("(keysym 0x0, NoSymbol)"), "Xlib observer: {line}");
        }
    }
    drop(xev);
    xlib_reader.join().unwrap();
    for mapping in &mut retained { mapping.restore().unwrap(); }
    let after = injector.conn.get_keyboard_mapping(first, count).unwrap().reply().unwrap();
    assert_eq!(after.keysyms_per_keycode, before.keysyms_per_keycode);
    assert_eq!(after.keysyms, before.keysyms, "the entire core map must be restored");
    let native_restored = native_before == dump();
    assert!(native_restored, "the full XKB map must be restored");
    eprintln!("lifetime={lifetime:?} presses={presses} MappingNotify={notifications} core_restored=true XKB_restored={native_restored} held_key_preserved=true delivered={}/{}", symbols.len(), repetitions);
    if lifetime == MappingLifetime::UntilTranslated || std::env::var_os("ONE_MK_REQUIRE_X11_TEXT_DELIVERY").is_some() {
        assert_eq!(symbols, expected, "a delayed client refreshes from the already-restored map");
    } else {
        assert!(symbols.is_empty(), "revisit the remap safety blocker if a delayed reader now receives text");
    }
}

struct TestXev(Child);

impl Drop for TestXev {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct TestMapping<'a> {
    conn: &'a RustConnection,
    keycode: u8,
    original: &'a [u32],
    active: bool,
}

impl TestMapping<'_> {
    fn restore(&mut self) -> Result<()> {
        self.conn.change_keyboard_mapping(1, self.keycode, u8::try_from(self.original.len())?, self.original)?.check()?;
        self.conn.sync()?;
        self.active = false;
        Ok(())
    }
}

impl Drop for TestMapping<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.restore();
        }
    }
}

#[test]
fn native_typing_snapshots_follow_server_maps_and_modifiers() {
    let Some(server) = Xvfb::start() else { return };
    let observer = server.observer();
    let mut keyboard = crate::X11Keyboard::connect(&server.display).expect("native XKB connection");
    for (layout, code, modifier, expected) in [
        ("de", 21, None, "z"),
        ("de", 44, None, "y"),
        ("de", 16, Some(100), "@"),
        ("fr", 16, None, "a"),
        ("fr", 11, Some(100), "@"),
        ("pl", 38, Some(100), "ł"),
        ("tr", 23, None, "ı"),
        ("us", 21, Some(42), "Y"),
    ] {
        assert!(Command::new("setxkbmap").args(["-display", &server.display, layout]).status().unwrap().success());
        if let Some(modifier) = modifier {
            observer.fake(xproto::KEY_PRESS_EVENT, modifier + 8, observer.root, 0, 0);
        }
        let snapshot = keyboard.snapshot().expect("current native keymap and state");
        assert_eq!(snapshot.resolver.resolve(code, snapshot.masks).as_deref(), Some(expected), "{layout}");
        assert_eq!(snapshot.resolver.reverse('z', snapshot.masks.group).unwrap().code, match layout { "de" => 21, "fr" => 17, _ => 44 });
        if let Some(modifier) = modifier {
            observer.fake(xproto::KEY_RELEASE_EVENT, modifier + 8, observer.root, 0, 0);
        }
    }
}

#[test]
fn native_typing_snapshot_preserves_the_old_map_during_a_hold() {
    let Some(server) = Xvfb::start() else { return };
    let observer = server.observer();
    assert!(Command::new("setxkbmap").args(["-display", &server.display, "de"]).status().unwrap().success());
    let mut keyboard = crate::X11Keyboard::connect(&server.display).unwrap();
    let before = keyboard.snapshot().unwrap();
    observer.fake(xproto::KEY_PRESS_EVENT, 29 + 8, observer.root, 0, 0);
    assert!(Command::new("setxkbmap").args(["-display", &server.display, "us"]).status().unwrap().success());
    let after = keyboard.snapshot().unwrap();
    assert_ne!(after.masks.depressed, 0, "native Ctrl state survives the map change");
    assert_eq!(before.resolver.resolve(21, before.masks).as_deref(), Some("z"));
    assert_eq!(after.resolver.resolve(21, crate::LayoutState::default()).as_deref(), Some("y"));
    observer.fake(xproto::KEY_RELEASE_EVENT, 29 + 8, observer.root, 0, 0);
    assert_eq!(keyboard.snapshot().unwrap().masks.depressed, 0);
}

#[test]
fn native_typing_snapshots_read_group_and_caps_lock() {
    let Some(server) = Xvfb::start() else { return };
    let observer = server.observer();
    assert!(Command::new("setxkbmap").args([
        "-display", &server.display, "-layout", "us,de", "-option", "", "-option", "grp:alt_shift_toggle",
    ]).status().unwrap().success());
    let mut keyboard = crate::X11Keyboard::connect(&server.display).unwrap();
    let initial = keyboard.snapshot().unwrap();
    assert_eq!(initial.masks.group, 0);
    assert_eq!(initial.resolver.resolve(21, initial.masks).as_deref(), Some("y"));
    for (code, down) in [(56, true), (42, true), (42, false), (56, false)] {
        observer.fake(if down { xproto::KEY_PRESS_EVENT } else { xproto::KEY_RELEASE_EVENT }, code + 8, observer.root, 0, 0);
    }
    let german = keyboard.snapshot().unwrap();
    assert_eq!(german.masks.group, 1);
    assert_eq!(german.resolver.resolve(21, german.masks).as_deref(), Some("z"));
    observer.fake(xproto::KEY_PRESS_EVENT, 58 + 8, observer.root, 0, 0);
    observer.fake(xproto::KEY_RELEASE_EVENT, 58 + 8, observer.root, 0, 0);
    let caps = keyboard.snapshot().unwrap();
    assert_ne!(caps.masks.locked, 0);
    assert_eq!(caps.resolver.resolve(21, caps.masks).as_deref(), Some("Z"));
    assert_eq!(initial.resolver.resolve(21, initial.masks).as_deref(), Some("y"));
}

/// All of the right edge of the default `Xvfb` screen.
fn right_edge() -> Edge {
    Edge::of(Side::Right, &Rect::new(0, 0, i32::from(W), i32::from(H)))
}

#[tokio::test]
async fn partial_right_edge_does_not_capture_the_wall() {
    for barriers in [true, false] {
        let Some(server) = Xvfb::start_sized(900) else { return };
        let observer = server.observer();
        observer.move_to(600, 100);
        let (mut backend, _locked) = open_target(Target { display: Some(server.display.clone()), barriers }).unwrap();
        assert!(matches!(next(&mut backend.events).await, Event::Desktop(_)));
        backend.capture.send(CaptureCmd::SetEdges(vec![Edge { side: Side::Right, at: i32::from(W), lo: 300, hi: 800 }])).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        for y in [100, 299, 800, 850] {
            observer.move_to(600, y);
            observer.move_to(W - 1, y);
            observer.move_by(10, 0);
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(nothing(&mut backend.events), "wall captured (barriers={barriers}, y={y})");
            assert!(observer.can_grab());
        }
        observer.move_to(600, 500);
        observer.move_to(W - 1, 500);
        observer.move_by(10, 0);
        let (id, side, pos) = activated(&mut backend.events).await;
        assert_eq!((side, pos), (Side::Right, Pos::new(f64::from(W - 1), 500.0)));
        backend.capture.send(CaptureCmd::Release { id, pos: Some(Pos::new(600.0, 500.0)) }).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        for _ in 0..100 {
            if observer.can_grab() { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(observer.can_grab(), "partial-edge grab not released (barriers={barriers}, pointer={:?}, event={:?})", observer.pointer(), backend.events.try_recv());
    }
}

/// A private X server on a free display, killed when dropped.
struct Xvfb {
    child: Child,
    display: String,
}

impl Xvfb {
    fn start() -> Option<Xvfb> {
        Self::start_sized(H)
    }

    fn start_sized(height: i16) -> Option<Xvfb> {
        // Xvfb picks a free display itself and tells us which.
        let child = Command::new("Xvfb")
            .args(["-displayfd", "1", "-screen", "0", &format!("{W}x{height}x24"), "-nolisten", "tcp", "-noreset"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else {
            eprintln!("Xvfb not installed: skipping");
            return None;
        };
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().expect("piped")).read_line(&mut line).unwrap();
        let display = format!(":{}", line.trim());
        assert!(line.trim().parse::<u32>().is_ok(), "Xvfb did not start: {line:?}");
        Some(Xvfb { child, display })
    }

    fn observer(&self) -> Observer {
        let (conn, screen) = x11rb::connect(Some(&self.display)).expect("observer connects");
        let root = conn.setup().roots[screen].root;
        conn.xtest_get_version(2, 2).unwrap().reply().unwrap();
        Observer { conn, root }
    }
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        // SIGTERM, so it removes its socket and lock file.
        // SAFETY: plain kill(2) on our own child.
        unsafe { libc::kill(self.child.id() as i32, libc::SIGTERM) };
        let _ = self.child.wait();
    }
}

/// A second X client: looks at what the backend did, and plays the local
/// mouse and keyboard through XTest.
struct Observer {
    conn: RustConnection,
    root: xproto::Window,
}

impl Observer {
    fn pointer(&self) -> (i16, i16, u16) {
        let p = self.conn.query_pointer(self.root).unwrap().reply().unwrap();
        (p.root_x, p.root_y, u16::from(p.mask))
    }

    fn key_down(&self, keycode: u8) -> bool {
        let keys = self.conn.query_keymap().unwrap().reply().unwrap().keys;
        keys[usize::from(keycode / 8)] & (1 << (keycode % 8)) != 0
    }

    /// Whether this client could grab the keyboard (and lets go again).
    fn can_grab(&self) -> bool {
        let m = xproto::GrabMode::ASYNC;
        let s = self.conn.grab_keyboard(false, self.root, CURRENT_TIME, m, m).unwrap().reply().unwrap().status;
        self.conn.ungrab_keyboard(CURRENT_TIME).unwrap();
        self.conn.flush().unwrap();
        s == GrabStatus::SUCCESS
    }

    fn fake(&self, type_: u8, detail: u8, root: xproto::Window, x: i16, y: i16) {
        self.conn.xtest_fake_input(type_, detail, CURRENT_TIME, root, x, y, 0).unwrap();
        self.conn.sync().unwrap();
    }

    fn move_to(&self, x: i16, y: i16) {
        self.fake(xproto::MOTION_NOTIFY_EVENT, 0, self.root, x, y);
    }

    /// Relative motion, as a mouse makes it (barriers only stop that).
    fn move_by(&self, dx: i16, dy: i16) {
        self.fake(xproto::MOTION_NOTIFY_EVENT, 1, NONE, dx, dy);
    }

    /// The device id of the core XTEST `kind` ("keyboard" or "pointer"):
    /// what this client's XTest input comes from.
    fn xtest(&self, kind: &str) -> DeviceRef {
        let name = format!("Virtual core XTEST {kind}");
        let infos = self.conn.xinput_xi_query_device(0u16).unwrap().reply().unwrap().infos;
        DeviceRef(u32::from(infos.iter().find(|d| d.name == name.as_bytes()).expect("XTEST device").deviceid))
    }

    fn xi_id(&self, name: &str) -> u16 {
        let infos = self.conn.xinput_xi_query_device(0u16).unwrap().reply().unwrap().infos;
        infos.iter().find(|d| d.name == name.as_bytes()).unwrap_or_else(|| panic!("no device {name}")).deviceid
    }

    /// Adds a master pointer and keyboard `name`, each with its own XTEST
    /// slave: as far as X is concerned, a keyboard and mouse plugged in.
    fn add_master(&self, name: &str) {
        let add = xinput::HierarchyChangeDataAddMaster { send_core: true, enable: true, name: name.as_bytes().to_vec() };
        let len = ((8 + name.len()).div_ceil(4)) as u16;
        let change = xinput::HierarchyChange { len, data: xinput::HierarchyChangeData::AddMaster(add) };
        self.conn.xinput_xi_change_hierarchy(&[change]).unwrap();
        self.conn.sync().unwrap();
    }

    /// Removes the master `name` and its XTEST slaves: unplugged.
    fn remove_master(&self, name: &str) {
        let remove = xinput::HierarchyChangeDataRemoveMaster {
            deviceid: self.xi_id(&format!("{name} pointer")),
            return_mode: xinput::ChangeMode::FLOAT,
            return_pointer: 0,
            return_keyboard: 0,
        };
        let change = xinput::HierarchyChange { len: 3, data: xinput::HierarchyChangeData::RemoveMaster(remove) };
        self.conn.xinput_xi_change_hierarchy(&[change]).unwrap();
        self.conn.sync().unwrap();
    }

    /// From now on this client's XTest input comes through the devices of
    /// master `name` (`Virtual core` for the first ones).
    fn type_through(&self, name: &str) {
        self.conn.xinput_xi_set_client_pointer(NONE, self.xi_id(&format!("{name} pointer"))).unwrap();
        self.conn.sync().unwrap();
    }

    fn sync(&self) {
        self.conn.sync().unwrap();
    }
}

trait Sync_ {
    fn sync(&self) -> Result<()>;
}

impl Sync_ for RustConnection {
    fn sync(&self) -> Result<()> {
        self.get_input_focus()?.reply()?;
        Ok(())
    }
}

/// Polls `f` until it holds (the backend works on its own thread).
fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

/// The next event that is not a status report or the list of devices.
async fn next(events: &mut mpsc::UnboundedReceiver<Event>) -> Event {
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .expect("an event within 3 s")
            .expect("backend alive");
        if !matches!(ev, Event::Status { .. } | Event::Devices(_)) {
            return ev;
        }
    }
}

/// Whether nothing but status reports and device lists is waiting.
fn nothing(events: &mut mpsc::UnboundedReceiver<Event>) -> bool {
    loop {
        match events.try_recv() {
            Err(_) => return true,
            Ok(Event::Status { .. } | Event::Devices(_)) => {}
            Ok(_) => return false,
        }
    }
}

async fn activated(events: &mut mpsc::UnboundedReceiver<Event>) -> (u32, Side, Pos) {
    loop {
        match next(events).await {
            Event::Activated { id, side, pos } => return (id, side, pos),
            Event::Modifiers(_) => {}
            ev => panic!("expected Activated, got {ev:?}"),
        }
    }
}

#[tokio::test]
async fn injection_reaches_the_x_server() {
    let Some(x) = Xvfb::start() else { return };
    let o = x.observer();
    let (mut b, _locked) = open_on(&x.display).unwrap();
    let screen = Desktop::new(vec![Rect::new(0, 0, W.into(), H.into())]);
    assert_eq!(next(&mut b.events).await, Event::Desktop(screen));

    b.inject.send(Inject::MoveTo(Pos::new(100.0, 200.0))).unwrap();
    eventually("the absolute move", || o.pointer().0 == 100 && o.pointer().1 == 200);
    b.inject.send(Inject::MoveBy { dx: 3, dy: -2 }).unwrap();
    eventually("the relative move", || o.pointer().0 == 103 && o.pointer().1 == 198);
    b.inject.send(Inject::MoveTo(Pos::new(99999.0, -5.0))).unwrap();
    eventually("a move clamped onto the screen", || o.pointer().0 == W - 1 && o.pointer().1 == 0);

    b.inject.send(Inject::Key { code: 30, down: true }).unwrap();
    eventually("A down (keycode 38)", || o.key_down(38));
    b.inject.send(Inject::Key { code: 30, down: false }).unwrap();
    eventually("A up", || !o.key_down(38));

    let button1 = u16::from(xproto::KeyButMask::BUTTON1);
    let button3 = u16::from(xproto::KeyButMask::BUTTON3);
    b.inject.send(Inject::Button { button: 0x110, down: true }).unwrap();
    eventually("left button down", || o.pointer().2 & button1 != 0);
    b.inject.send(Inject::Button { button: 0x110, down: false }).unwrap();
    b.inject.send(Inject::Button { button: 0x111, down: true }).unwrap();
    eventually("left up, right down", || o.pointer().2 & (button1 | button3) == button3);
    b.inject.send(Inject::Button { button: 0x111, down: false }).unwrap();
    eventually("right up", || o.pointer().2 & button3 == 0);

    // Wheel clicks arrive as buttons 4-7 in the window under the pointer.
    let win = o.conn.generate_id().unwrap();
    let aux = xproto::CreateWindowAux::new().override_redirect(1).event_mask(xproto::EventMask::BUTTON_PRESS);
    o.conn
        .create_window(0, win, o.root, 0, 0, W as u16, H as u16, 0, xproto::WindowClass::INPUT_ONLY, 0, &aux)
        .unwrap();
    o.conn.map_window(win).unwrap();
    o.sync();
    b.inject.send(Inject::MoveTo(Pos::new(500.0, 500.0))).unwrap();
    for m in [
        Inject::ScrollDiscrete { dx: 0, dy: 120 },
        Inject::ScrollDiscrete { dx: 0, dy: -60 },
        Inject::ScrollDiscrete { dx: 0, dy: -60 },
        Inject::ScrollDiscrete { dx: 120, dy: 0 },
        Inject::Scroll { dx: -30.0, dy: 0.0 },
        Inject::Scroll { dx: -30.0, dy: 0.0 },
    ] {
        b.inject.send(m).unwrap();
        // One at a time, so they are not coalesced.
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut buttons = Vec::new();
    eventually("four wheel clicks", || {
        while let Some(ev) = o.conn.poll_for_event().unwrap() {
            if let x11rb::protocol::Event::ButtonPress(e) = ev {
                buttons.push(e.detail);
            }
        }
        buttons.len() >= 4
    });
    assert_eq!(buttons, vec![5, 4, 7, 6]);

    // A key still held when the daemon goes away is let go of.
    b.inject.send(Inject::Key { code: 42, down: true }).unwrap();
    eventually("Shift down", || o.key_down(50));
    drop(b);
    eventually("Shift let go of when the daemon went away", || !o.key_down(50));
}

#[tokio::test]
async fn pushing_against_an_edge_captures_until_released() {
    let Some(x) = Xvfb::start() else { return };
    let o = x.observer();
    let (mut b, locked) = open_on(&x.display).unwrap();
    assert!(matches!(next(&mut b.events).await, Event::Desktop(_)));
    b.capture.send(CaptureCmd::SetEdges(vec![right_edge()])).unwrap();

    // Along the edge without pushing out: nothing.
    o.move_to(W - 1, 400);
    o.move_by(0, 5);
    // The left edge has no peer.
    o.move_to(0, 400);
    o.move_by(-10, 0);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(nothing(&mut b.events), "nothing captured yet");
    assert!(o.can_grab());

    o.move_to(W - 1, 400);
    o.move_by(10, 0);
    let (id, side, pos) = activated(&mut b.events).await;
    assert_eq!((side, pos), (Side::Right, Pos::new(f64::from(W - 1), 400.0)));
    assert!(!o.can_grab(), "the keyboard is ours");

    // Local input now goes to the peer, and nowhere here.
    o.move_by(-3, 2);
    assert_eq!(next(&mut b.events).await, Event::Motion { dx: -3.0, dy: 2.0, device: Some(o.xtest("pointer")) });
    assert_eq!((o.pointer().0, o.pointer().1), (W - 1, 400), "the pointer stays put");
    o.fake(2, 38, NONE, 0, 0);
    o.fake(3, 38, NONE, 0, 0);
    assert_eq!(next(&mut b.events).await, Event::Key { code: 30, down: true, device: Some(o.xtest("keyboard")) });
    assert_eq!(next(&mut b.events).await, Event::Key { code: 30, down: false, device: Some(o.xtest("keyboard")) });
    o.fake(4, 1, NONE, 0, 0);
    o.fake(5, 1, NONE, 0, 0);
    assert_eq!(next(&mut b.events).await, Event::Button { button: 0x110, down: true, device: Some(o.xtest("pointer")) });
    assert_eq!(next(&mut b.events).await, Event::Button { button: 0x110, down: false, device: Some(o.xtest("pointer")) });
    o.fake(4, 5, NONE, 0, 0);
    o.fake(5, 5, NONE, 0, 0);
    assert_eq!(next(&mut b.events).await, Event::ScrollDiscrete { dx: 0, dy: 120, device: Some(o.xtest("pointer")) });

    // A release for an older activation is ignored; ours lets go.
    b.capture.send(CaptureCmd::Release { id: id.wrapping_sub(1), pos: None }).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!o.can_grab());
    b.capture.send(CaptureCmd::Release { id, pos: Some(Pos::new(1270.0, 300.0)) }).unwrap();
    eventually("the pointer put back", || (o.pointer().0, o.pointer().1) == (1270, 300));
    assert!(o.can_grab(), "the keyboard is free again");
    o.move_by(-5, 0);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let ev = b.events.try_recv();
    assert!(ev.is_err(), "local input stays local: {ev:?}");

    // `one-mk switch`, then the screen locks: capture ends by itself.
    b.capture.send(CaptureCmd::Activate { side: Side::Left }).unwrap();
    let (id, side, _) = activated(&mut b.events).await;
    assert_eq!(side, Side::Left);
    assert!(!o.can_grab());
    locked.send(true).unwrap();
    assert_eq!(next(&mut b.events).await, Event::Deactivated { id });
    eventually("the grab released on lock", || o.can_grab());
    o.move_to(W - 1, 100);
    o.move_by(10, 0);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(nothing(&mut b.events), "no capture while locked");
    b.capture.send(CaptureCmd::Activate { side: Side::Right }).unwrap();
    assert!(matches!(next(&mut b.events).await, Event::Error(_)));
    locked.send(false).unwrap();

    // While a peer controls this machine, local input is reported, and
    // our own injected input is not.
    b.inject.send(Inject::Controlled(true)).unwrap();
    b.inject.send(Inject::MoveBy { dx: -2, dy: 0 }).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(nothing(&mut b.events), "injected motion is not local input");
    // (XTest is all this X server has: the observer's own XTest device
    // counts as ours, so this is as far as Xvfb lets us check.)

    // Another program holding the keyboard: no capture, and no retry
    // until the pointer leaves the edge.
    b.inject.send(Inject::Controlled(false)).unwrap();
    let m = xproto::GrabMode::ASYNC;
    o.conn.grab_keyboard(false, o.root, CURRENT_TIME, m, m).unwrap().reply().unwrap();
    o.move_to(W - 1, 200);
    o.move_by(10, 0);
    o.move_by(10, 0);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(nothing(&mut b.events), "no capture while another client has the keyboard");
    o.conn.ungrab_keyboard(CURRENT_TIME).unwrap();
    o.move_by(-20, 0);
    o.move_by(30, 0);
    let (id, _, _) = activated(&mut b.events).await;

    // The daemon going away ends capture.
    drop(b.capture);
    drop(b.inject);
    eventually("the grab released when the daemon went away", || o.can_grab());
    let _ = id;
}

#[tokio::test]
async fn without_barriers_an_edge_window_captures() {
    let Some(x) = Xvfb::start() else { return };
    let o = x.observer();
    let (mut b, _locked) = open_target(Target { display: Some(x.display.clone()), barriers: false }).unwrap();
    assert!(matches!(next(&mut b.events).await, Event::Desktop(_)));
    o.move_to(W - 1, 400);
    b.capture.send(CaptureCmd::SetEdges(vec![right_edge()])).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(nothing(&mut b.events), "the pointer was there before: no capture");
    o.move_to(600, 400);
    o.move_to(W - 1, 400);
    let (id, side, pos) = activated(&mut b.events).await;
    assert_eq!((side, pos), (Side::Right, Pos::new(f64::from(W - 1), 400.0)));
    assert!(!o.can_grab());
    o.move_by(-4, 1);
    assert_eq!(next(&mut b.events).await, Event::Motion { dx: -4.0, dy: 1.0, device: Some(o.xtest("pointer")) });

    // Put back onto the edge: no capture until it has left the edge.
    b.capture.send(CaptureCmd::Release { id, pos: Some(Pos::new(f64::from(W - 1), 300.0)) }).unwrap();
    eventually("the pointer put back", || (o.pointer().0, o.pointer().1) == (W - 1, 300));
    assert!(o.can_grab());
    o.move_to(W - 1, 310);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(nothing(&mut b.events), "no capture while still on the edge");
    o.move_to(W - 10, 310);
    o.move_to(W - 1, 320);
    let (_, side, _) = activated(&mut b.events).await;
    assert_eq!(side, Side::Right);
}

#[tokio::test]
async fn monitor_changes_are_reported() {
    let Some(x) = Xvfb::start() else { return };
    let (mut b, _locked) = open_on(&x.display).unwrap();
    assert!(matches!(next(&mut b.events).await, Event::Desktop(_)));
    // Xvfb can only shrink its screen (which also turns its one output
    // off); that is a monitor change all the same.
    let xrandr = |args: &[&str]| {
        Command::new("xrandr").arg("-d").arg(&x.display).args(args).stderr(Stdio::null()).status().is_ok()
    };
    b.capture.send(CaptureCmd::Activate { side: Side::Top }).unwrap();
    let (id, ..) = activated(&mut b.events).await;
    if !xrandr(&["--fb", "1024x768"]) {
        eprintln!("xrandr not installed: skipping");
        return;
    }
    // Capture in progress ends: the screen it started on may be gone.
    assert_eq!(next(&mut b.events).await, Event::Deactivated { id });
    assert_eq!(next(&mut b.events).await, Event::Desktop(Desktop::new(vec![Rect::new(0, 0, 1024, 768)])));
    assert!(x.observer().can_grab());
}

/// Run by `kill_9_releases_the_grab` in a process of its own: captures,
/// presses a key, and waits to be killed.
#[tokio::test]
#[ignore = "run by kill_9_releases_the_grab"]
async fn child_captures_and_waits() {
    let Ok(display) = std::env::var("ONE_MK_TEST_DISPLAY") else { return };
    let (mut b, _locked) = open_on(&display).unwrap();
    assert!(matches!(next(&mut b.events).await, Event::Desktop(_)));
    b.inject.send(Inject::Key { code: 30, down: true }).unwrap();
    b.capture.send(CaptureCmd::Activate { side: Side::Right }).unwrap();
    activated(&mut b.events).await;
    println!("CAPTURED");
    tokio::time::sleep(Duration::from_secs(60)).await;
}

#[test]
fn kill_9_releases_the_grab() {
    let Some(x) = Xvfb::start() else { return };
    let o = x.observer();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "x11::tests::child_captures_and_waits", "--ignored", "--nocapture", "--test-threads=1"])
        .env("ONE_MK_TEST_DISPLAY", &x.display)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let out = std::io::BufReader::new(child.stdout.take().unwrap());
    let captured = out.lines().map_while(Result::ok).any(|l| l.contains("CAPTURED"));
    assert!(captured, "the child captured");
    assert!(!o.can_grab(), "the child holds the keyboard");
    assert!(o.key_down(38), "the child holds A down");
    // SAFETY: plain kill(2) on our own child.
    unsafe { libc::kill(child.id() as i32, libc::SIGKILL) };
    child.wait().unwrap();
    eventually("the grab gone with the killed process", || o.can_grab());
    // XTest keeps keys pressed after the client that pressed them is gone;
    // the next start lets go of them.
    assert!(o.key_down(38), "XTest still holds A after kill -9");
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let (_b, _locked) = rt.block_on(async { open_on(&x.display) }).unwrap();
    eventually("A let go of by the next start", || !o.key_down(38));
}

/// The next list of devices for which `want` holds.
async fn devices_where(events: &mut mpsc::UnboundedReceiver<Event>, want: impl Fn(&[InputDevice]) -> bool) -> Vec<InputDevice> {
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(3), events.recv()).await.expect("devices within 3 s").expect("backend alive");
        if let Event::Devices(list) = ev
            && want(&list)
        {
            return list;
        }
    }
}

/// Xvfb has no real keyboard or mouse: only X's own XTEST devices, which
/// stand in for them here, and its own built-in ones (no device node, so
/// their connection is unknown). They are listed, input says
/// which one it came from, and a master added and removed (as a device
/// plugged in and out) shows up and goes away.
#[tokio::test]
async fn devices_are_listed_and_input_says_which_one() {
    let Some(x) = Xvfb::start() else { return };
    let o = x.observer();
    let (mut b, _locked) = open_on(&x.display).unwrap();
    let list = devices_where(&mut b.events, |_| true).await;
    let names: Vec<&str> = list.iter().map(|d| d.fingerprint.name.as_str()).collect();
    assert_eq!(names, ["Virtual core XTEST pointer", "Virtual core XTEST keyboard", "Xvfb mouse", "Xvfb keyboard"]);
    let buses: Vec<Bus> = list.iter().map(|d| d.fingerprint.bus).collect();
    assert_eq!(buses, [Bus::Virtual, Bus::Virtual, Bus::Unknown, Bus::Unknown]);
    let keyboard = list.iter().find(|d| d.kinds.keyboard).unwrap();
    assert_eq!(keyboard.id, o.xtest("keyboard"));
    assert_eq!(keyboard.fingerprint.to_string(), "virtual/0000:0000//Virtual core XTEST keyboard");

    o.add_master("second");
    let list = devices_where(&mut b.events, |l| l.len() == 6).await;
    let second = list.iter().find(|d| d.fingerprint.name == "second XTEST keyboard").expect("plugged in").id;
    assert_eq!(second, DeviceRef(u32::from(o.xi_id("second XTEST keyboard"))));

    b.capture.send(CaptureCmd::SetEdges(vec![right_edge()])).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    o.move_to(W - 1, 400);
    o.move_by(10, 0);
    activated(&mut b.events).await;
    o.fake(2, 38, NONE, 0, 0);
    o.fake(3, 38, NONE, 0, 0);
    assert_eq!(next(&mut b.events).await, Event::Key { code: 30, down: true, device: Some(keyboard.id) });
    assert_eq!(next(&mut b.events).await, Event::Key { code: 30, down: false, device: Some(keyboard.id) });
    o.type_through("second");
    o.fake(2, 38, NONE, 0, 0);
    o.fake(3, 38, NONE, 0, 0);
    assert_eq!(next(&mut b.events).await, Event::Key { code: 30, down: true, device: Some(second) });
    assert_eq!(next(&mut b.events).await, Event::Key { code: 30, down: false, device: Some(second) });

    // Unplugged while its key is down: it leaves the list (the daemon lets
    // go of what it held), and a press of the same key later on another
    // device is a press, not a repeat.
    o.fake(2, 38, NONE, 0, 0);
    assert_eq!(next(&mut b.events).await, Event::Key { code: 30, down: true, device: Some(second) });
    o.type_through("Virtual core");
    o.remove_master("second");
    let list = devices_where(&mut b.events, |l| l.len() == 4).await;
    assert!(list.iter().all(|d| !d.fingerprint.name.starts_with("second")));
    o.fake(2, 38, NONE, 0, 0);
    o.fake(3, 38, NONE, 0, 0);
    let mut rest = Vec::new();
    while rest.len() < 2 {
        match next(&mut b.events).await {
            Event::Key { code: 30, device, down } => rest.push((down, device)),
            ev => panic!("unexpected {ev:?}"),
        }
    }
    assert_eq!(rest.last(), Some(&(false, Some(keyboard.id))), "{rest:?}");
}
