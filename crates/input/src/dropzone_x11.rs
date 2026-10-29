use super::{Drop, Zone, MAX_URI_BYTES, uri_list};
use anyhow::{Context, Result};
use one_mk_proto::{Pos, drag::{Change, Drag}};
use std::os::fd::AsRawFd;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use x11rb::{NONE, COPY_DEPTH_FROM_PARENT};
use x11rb::connection::Connection;
use x11rb::protocol::{Event, xinput::{self, ConnectionExt as _}, xproto::{self, ConnectionExt as _}};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub(super) fn start(zones: watch::Receiver<Vec<Zone>>, dropped: mpsc::Sender<Drop>) -> Result<()> {
    let (conn, screen) = x11rb::connect(None)?;
    crate::linux_rt::spawn_thread("one-mk-drop", move || async move {
        if let Err(error) = run(conn, screen, zones, dropped).await {
            tracing::warn!("file drop target stopped: {error:#}");
        }
    })
}

struct Atoms {
    enter: u32, position: u32, status: u32, leave: u32, drop: u32, finished: u32,
    selection: u32, types: u32, uri: u32, copy: u32, property: u32, aware: u32,
}

impl Atoms {
    fn new(conn: &RustConnection) -> Result<Self> {
        let atom = |name: &[u8]| -> Result<u32> { Ok(conn.intern_atom(false, name)?.reply()?.atom) };
        Ok(Self {
            enter: atom(b"XdndEnter")?, position: atom(b"XdndPosition")?, status: atom(b"XdndStatus")?,
            leave: atom(b"XdndLeave")?, drop: atom(b"XdndDrop")?, finished: atom(b"XdndFinished")?,
            selection: atom(b"XdndSelection")?, types: atom(b"XdndTypeList")?, uri: atom(b"text/uri-list")?,
            copy: atom(b"XdndActionCopy")?, property: atom(b"ONE_MK_DROP")?, aware: atom(b"XdndAware")?,
        })
    }
}

struct Source { window: u32, uri: bool, positioned: bool }
struct Pending { source: u32, peer: String, time: u32 }

struct Target {
    conn: RustConnection,
    root: u32,
    window: u32,
    atoms: Atoms,
    zones: Vec<Zone>,
    state: Drag,
    shown: Option<usize>,
    held: bool,
    source: Option<Source>,
    pending: Option<Pending>,
    deadline: Option<Instant>,
    motion_at: Option<Instant>,
    physical: Vec<u16>,
}

impl Target {
    fn new(conn: RustConnection, screen: usize) -> Result<Self> {
        let root = conn.setup().roots[screen].root;
        conn.xinput_xi_query_version(2, 1)?.reply()?;
        let atoms = Atoms::new(&conn)?;
        let window = conn.generate_id()?;
        conn.create_window(COPY_DEPTH_FROM_PARENT, window, root, 0, 0, 1, 1, 0,
            xproto::WindowClass::INPUT_OUTPUT, 0,
            &xproto::CreateWindowAux::new().override_redirect(1).background_pixel(0x2674bd)
                .event_mask(xproto::EventMask::PROPERTY_CHANGE))?.check()?;
        conn.change_property32(xproto::PropMode::REPLACE, window, atoms.aware, xproto::AtomEnum::ATOM, &[5])?;
        let mut target = Self { conn, root, window, atoms, zones: vec![], state: Drag::default(), shown: None,
            held: false, source: None, pending: None, deadline: None, motion_at: None, physical: vec![] };
        target.devices()?;
        target.select()?;
        target.conn.flush()?;
        Ok(target)
    }

    fn select(&mut self) -> Result<()> {
        let mut mask = xinput::XIEventMask::RAW_BUTTON_PRESS | xinput::XIEventMask::RAW_BUTTON_RELEASE;
        if self.held { mask |= xinput::XIEventMask::RAW_MOTION | xinput::XIEventMask::RAW_KEY_PRESS | xinput::XIEventMask::MOTION; }
        if self.zones.is_empty() { mask = xinput::XIEventMask::default(); }
        let hierarchy = if self.zones.is_empty() { xinput::XIEventMask::default() } else { xinput::XIEventMask::HIERARCHY };
        self.conn.xinput_xi_select_events(self.root, &[
            xinput::EventMask { deviceid: 1, mask: vec![mask] },
            xinput::EventMask { deviceid: 0, mask: vec![hierarchy] },
        ])?;
        Ok(())
    }

    fn devices(&mut self) -> Result<()> {
        self.physical.clear();
        let devices = self.conn.xinput_xi_query_device(0u16)?.reply()?;
        self.physical.extend(devices.infos.into_iter().filter(physical_device).map(|device| device.deviceid));
        Ok(())
    }

    fn physical_event(&self, source: u16, response: u8) -> bool {
        response & 0x80 == 0 && self.physical.contains(&source)
    }

    fn hide(&mut self) -> Result<()> {
        if self.shown.take().is_some() { self.conn.unmap_window(self.window)?; }
        self.source = None;
        Ok(())
    }

    fn pointer(&mut self) -> Result<()> {
        if self.pending.is_some() { return Ok(()); }
        let pointer = self.conn.query_pointer(self.root)?.reply()?;
        let pos = Pos::new(pointer.root_x.into(), pointer.root_y.into());
        self.position(pos)
    }

    fn position(&mut self, pos: Pos) -> Result<()> {
        if self.pending.is_some() { return Ok(()); }
        let zone = self.zones.iter().position(|zone| zone.rect.contains(pos));
        match self.state.pointer(self.held, zone) {
            Change::Show(index) => {
                self.hide()?;
                let rect = self.zones[index].rect;
                self.conn.configure_window(self.window, &xproto::ConfigureWindowAux::new()
                    .x(rect.x).y(rect.y).width(rect.w as u32).height(rect.h as u32)
                    .stack_mode(xproto::StackMode::ABOVE))?;
                self.conn.map_window(self.window)?;
                self.shown = Some(index);
                self.deadline = Some(Instant::now() + Duration::from_secs(30));
            }
            Change::Hide => { self.hide()?; self.deadline = None; }
            Change::None => {}
        }
        Ok(())
    }

    fn message(&self, window: u32, kind: u32, data: [u32; 5]) -> Result<()> {
        self.conn.send_event(false, window, xproto::EventMask::NO_EVENT,
            xproto::ClientMessageEvent::new(32, window, kind, data))?;
        Ok(())
    }

    fn finish(&self, source: u32, accepted: bool) -> Result<()> {
        self.message(source, self.atoms.finished, [self.window, u32::from(accepted), if accepted { self.atoms.copy } else { NONE }, 0, 0])
    }

    fn cancel(&mut self) -> Result<()> {
        if let Some(pending) = self.pending.take() { self.finish(pending.source, false)?; }
        self.state.cancel();
        self.hide()?;
        self.deadline = None;
        self.motion_at = None;
        Ok(())
    }

    fn configure(&mut self, zones: Vec<Zone>) -> Result<()> {
        self.cancel()?;
        self.held = false;
        self.zones = zones;
        self.select()
    }

    fn event(&mut self, event: Event, dropped: &mpsc::Sender<Drop>) -> Result<()> {
        if let Err(error) = self.handle(event, dropped) {
            tracing::debug!("aborting file drop after X error: {error:#}");
            self.pending = None;
            self.cancel()?;
        }
        Ok(())
    }

    fn handle(&mut self, event: Event, dropped: &mpsc::Sender<Drop>) -> Result<()> {
        match event {
            Event::XinputHierarchy(_) => { self.cancel()?; self.held = false; self.devices()?; self.select()?; }
            Event::Error(error) => { anyhow::bail!("{error:?}"); }
            Event::XinputRawButtonPress(event) if event.detail == 1 && self.physical_event(event.sourceid, event.response_type) => {
                self.cancel()?;
                self.state.pointer(false, None);
                self.held = true;
                self.select()?;
                self.pointer()?;
            }
            Event::XinputRawButtonRelease(event) if event.detail == 1 && self.physical_event(event.sourceid, event.response_type) => {
                self.held = false;
                self.motion_at = None;
                self.select()?;
                if self.shown.is_some() && self.pending.is_none() {
                    self.deadline = Some(Instant::now() + Duration::from_millis(300));
                } else if self.pending.is_none() { self.pointer()?; }
            }
            Event::XinputRawMotion(event) if self.held && self.physical_event(event.sourceid, event.response_type) => {
                if self.motion_at.is_none() { self.motion_at = Some(Instant::now() + Duration::from_millis(16)); }
            }
            Event::XinputMotion(event) if self.held && self.physical_event(event.sourceid, event.response_type) => {
                self.motion_at = None;
                self.position(Pos::new(f64::from(event.root_x) / 65536., f64::from(event.root_y) / 65536.))?;
            }
            Event::XinputRawKeyPress(event) if event.detail == 9 && self.physical_event(event.sourceid, event.response_type) => self.cancel()?,
            Event::ClientMessage(event) if event.window == self.window && event.format == 32 => {
                let data = event.data.as_data32();
                if event.type_ == self.atoms.enter && self.held && self.shown.is_some() && self.pending.is_none() {
                    self.source = None;
                    let version = data[1] >> 24;
                    let offered = if data[1] & 1 == 0 { data[2..].contains(&self.atoms.uri) } else {
                        let types = self.conn.get_property(false, data[0], self.atoms.types, xproto::AtomEnum::ATOM, 0, 64)?.reply()?;
                        types.bytes_after == 0 && types.value32().is_some_and(|mut types| types.any(|atom| atom == self.atoms.uri))
                    };
                    self.source = Some(Source { window: data[0], uri: offered && (3..=5).contains(&version), positioned: false });
                } else if event.type_ == self.atoms.position {
                    if let Some(source) = &mut self.source && source.window == data[0] {
                        let pos = Pos::new((data[2] >> 16) as u16 as i16 as f64, data[2] as u16 as i16 as f64);
                        source.positioned = self.shown.is_some_and(|index| self.zones[index].rect.contains(pos));
                        let accept = source.uri && source.positioned;
                        self.message(data[0], self.atoms.status, [self.window, if accept { 3 } else { 2 }, 0, 0, if accept { self.atoms.copy } else { NONE }])?;
                    }
                } else if event.type_ == self.atoms.leave && self.source.as_ref().is_some_and(|source| source.window == data[0]) {
                    self.cancel()?;
                } else if event.type_ == self.atoms.drop {
                    let valid = self.source.as_ref().is_some_and(|source| source.window == data[0] && source.uri && source.positioned)
                        && self.conn.get_selection_owner(self.atoms.selection)?.reply()?.owner == data[0];
                    if valid && let Some(index) = self.shown && self.state.drop_on(index).is_some() {
                        self.pending = Some(Pending { source: data[0], peer: self.zones[index].peer.clone(), time: data[2] });
                        self.conn.delete_property(self.window, self.atoms.property)?;
                        self.conn.convert_selection(self.window, self.atoms.selection, self.atoms.uri, self.atoms.property, data[2])?;
                        self.hide()?;
                        self.deadline = Some(Instant::now() + Duration::from_secs(2));
                    } else { self.finish(data[0], false)?; }
                }
            }
            Event::SelectionNotify(event) if event.requestor == self.window && event.selection == self.atoms.selection
                && self.pending.as_ref().is_some_and(|pending| pending.time == event.time) => {
                    let pending = self.pending.take().context("pending drop disappeared")?;
                    let paths = if event.property == self.atoms.property && event.target == self.atoms.uri {
                        let reply = self.conn.get_property(true, self.window, self.atoms.property, self.atoms.uri, 0, (MAX_URI_BYTES / 4) as u32)?.reply()?;
                        if reply.type_ == self.atoms.uri && reply.format == 8 && reply.bytes_after == 0 { uri_list(&reply.value).ok() } else { None }
                    } else { None };
                    let accepted = paths.is_some_and(|paths| dropped.try_send(Drop { peer: pending.peer, paths }).is_ok());
                    self.finish(pending.source, accepted)?;
                    self.deadline = None;
                    if !self.held { self.state.pointer(false, None); }
            }
            _ => {}
        }
        Ok(())
    }
}

fn physical_device(device: &xinput::XIDeviceInfo) -> bool {
    let name = String::from_utf8_lossy(&device.name).to_ascii_lowercase();
    device.enabled && matches!(device.type_, xinput::DeviceType::SLAVE_POINTER | xinput::DeviceType::SLAVE_KEYBOARD)
        && !name.contains("xtest") && !name.contains("virtual")
}

async fn run(conn: RustConnection, screen: usize, mut zones: watch::Receiver<Vec<Zone>>, dropped: mpsc::Sender<Drop>) -> Result<()> {
    let mut target = Target::new(conn, screen)?;
    target.zones = zones.borrow_and_update().clone();
    target.select()?;
    run_target(target, zones, dropped).await
}

async fn run_target(mut target: Target, mut zones: watch::Receiver<Vec<Zone>>, dropped: mpsc::Sender<Drop>) -> Result<()> {
    let socket = unsafe { crate::linux_rt::Socket::new(target.conn.stream().as_raw_fd())? };
    loop {
        while let Some(event) = target.conn.poll_for_event()? { target.event(event, &dropped)?; }
        target.conn.flush()?;
        let deadline = target.deadline.into_iter().chain(target.motion_at).min();
        tokio::select! {
            ready = socket.readable() => ready?,
            changed = zones.changed() => {
                if changed.is_err() { return Ok(()); }
                target.configure(zones.borrow_and_update().clone())?;
            }
            _ = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                if target.deadline.is_some_and(|deadline| deadline <= Instant::now()) {
                    target.cancel()?;
                    if !target.held { target.state.pointer(false, None); }
                } else {
                    target.motion_at = None;
                    if let Err(error) = target.pointer() {
                        tracing::debug!("aborting file drop after pointer error: {error:#}");
                        target.cancel()?;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::path::PathBuf;
    use x11rb::protocol::xtest::ConnectionExt as _;

    struct Server(Child);
    impl std::ops::Drop for Server {
        fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
    }

    async fn event(conn: &RustConnection, wanted: impl Fn(&Event) -> bool) -> Event {
        tokio::time::timeout(Duration::from_secs(5), async {
            let socket = unsafe { crate::linux_rt::Socket::new(conn.stream().as_raw_fd()).unwrap() };
            loop {
                while let Some(event) = conn.poll_for_event().unwrap() {
                    if wanted(&event) { return event; }
                }
                socket.readable().await.unwrap();
            }
        }).await.expect("scripted XDND source timed out")
    }

    #[tokio::test]
    async fn private_xvfb_xtest_cannot_arm_and_bad_enter_is_recoverable() {
        let child = Command::new("Xvfb").args(["-displayfd", "1", "-screen", "0", "800x600x24", "-nolisten", "tcp"])
            .stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let mut server = Server(child);
        let mut display = String::new();
        BufReader::new(server.0.stdout.take().unwrap()).read_line(&mut display).unwrap();
        let display = format!(":{}", display.trim());
        let (connection, screen) = x11rb::connect(Some(&display)).unwrap();
        let mut target = Target::new(connection, screen).unwrap();
        target.configure(vec![Zone { rect: one_mk_proto::Rect::new(788, 0, 12, 600), peer: "paired".into() }]).unwrap();
        let (sender, _receiver) = mpsc::channel(1);
        target.conn.flush().unwrap();
        let (attacker, _) = x11rb::connect(Some(&display)).unwrap();
        attacker.xtest_fake_input(xproto::BUTTON_PRESS_EVENT, 1, 0, target.root, 0, 0, 0).unwrap();
        attacker.xtest_fake_input(xproto::MOTION_NOTIFY_EVENT, 0, 0, target.root, 795, 300, 0).unwrap();
        attacker.get_input_focus().unwrap().reply().unwrap();
        target.conn.get_input_focus().unwrap().reply().unwrap();
        while let Some(event) = target.conn.poll_for_event().unwrap() { target.event(event, &sender).unwrap(); }
        assert!(!target.held);
        assert!(target.shown.is_none());
        let devices = target.conn.xinput_xi_query_device(0u16).unwrap().reply().unwrap();
        let xtest = devices.infos.iter().find(|device| device.name == b"Virtual core XTEST pointer").unwrap().deviceid;
        target.event(Event::XinputRawButtonPress(xinput::RawButtonPressEvent { sourceid: xtest, detail: 1, ..Default::default() }), &sender).unwrap();
        let injected_held = target.held;
        target.held = true;
        target.conn.warp_pointer(NONE, target.root, 0, 0, 0, 0, 795, 300).unwrap();
        target.pointer().unwrap();
        target.held = false;
        target.event(Event::ClientMessage(xproto::ClientMessageEvent::new(32, target.window, target.atoms.enter,
            [0xffff_fffe, 5 << 24, target.atoms.uri, 0, 0])), &sender).unwrap();
        assert!(target.source.is_none(), "Enter without a held physical button is not authorized");
        target.held = true;
        let bad = Event::ClientMessage(xproto::ClientMessageEvent::new(32, target.window, target.atoms.enter, [0xffff_fffe, (5 << 24) | 1, 0, 0, 0]));
        assert!(target.event(bad, &sender).is_ok(), "BadWindow must abort only the current drag");
        assert!(!injected_held, "XTEST must not arm a file drop");
        target.physical = vec![99];
        target.event(Event::XinputRawButtonPress(xinput::RawButtonPressEvent { sourceid: 99, detail: 1, ..Default::default() }), &sender).unwrap();
        assert!(target.shown.is_some(), "a subsequent physical drag still works after BadWindow");
        target.event(Event::XinputRawMotion(xinput::RawMotionEvent { sourceid: 99, ..Default::default() }), &sender).unwrap();
        let motion_at = target.motion_at;
        for _ in 0..100 {
            target.event(Event::XinputRawMotion(xinput::RawMotionEvent { sourceid: 99, ..Default::default() }), &sender).unwrap();
            assert_eq!(target.motion_at, motion_at, "raw motion coalesces without synchronous pointer queries");
        }
        target.event(Event::XinputRawButtonRelease(xinput::RawButtonReleaseEvent { sourceid: 99, detail: 1, ..Default::default() }), &sender).unwrap();
        assert!(target.motion_at.is_none(), "release disarms motion sampling");
        target.configure(vec![]).unwrap();
        assert!(target.deadline.is_none());
    }

    #[test]
    fn device_filter_accepts_only_enabled_nonvirtual_slaves() {
        let mut device = xinput::XIDeviceInfo { deviceid: 99, enabled: true, type_: xinput::DeviceType::SLAVE_POINTER,
            name: b"USB mouse".to_vec(), attachment: 2, classes: vec![] };
        assert!(physical_device(&device));
        for name in ["Virtual core XTEST pointer", "Virtual core pointer", "virtual tablet", "XTEST keyboard"] {
            device.name = name.as_bytes().to_vec();
            assert!(!physical_device(&device));
        }
        device.name = b"USB mouse".to_vec();
        device.enabled = false;
        assert!(!physical_device(&device));
        device.enabled = true;
        device.type_ = xinput::DeviceType::MASTER_POINTER;
        assert!(!physical_device(&device));
    }

    #[tokio::test]
    async fn private_xvfb_scripted_physical_drag_selection_and_copy_drop() {
        let child = Command::new("Xvfb").args(["-displayfd", "1", "-screen", "0", "800x600x24", "-nolisten", "tcp"])
            .stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("Xvfb required for drop-path test");
        let mut server = Server(child);
        let mut display = String::new();
        BufReader::new(server.0.stdout.take().unwrap()).read_line(&mut display).unwrap();
        let display = format!(":{}", display.trim());
        let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
        let mut target = Target::new(conn, screen).unwrap();
        target.held = true;
        target.configure(vec![]).unwrap();
        assert!(!target.held, "reconfiguration must forget buttons released while unsubscribed");
        let window = target.window;
        target.zones = vec![Zone { rect: one_mk_proto::Rect::new(788, 0, 12, 600), peer: "paired-id".into() }];
        target.physical = vec![99];
        target.select().unwrap();
        let (drop_tx, mut drops) = mpsc::channel(1);
        target.event(Event::XinputRawButtonPress(xinput::RawButtonPressEvent { sourceid: 99, detail: 1, ..Default::default() }), &drop_tx).unwrap();
        target.event(Event::XinputMotion(xinput::MotionEvent { sourceid: 99, root_x: 795 << 16, root_y: 300 << 16, ..Default::default() }), &drop_tx).unwrap();
        target.conn.flush().unwrap();
        let (zones_tx, zones_rx) = watch::channel(target.zones.clone());
        let worker = tokio::spawn(run_target(target, zones_rx, drop_tx));
        let (source, screen) = x11rb::connect(Some(&display)).unwrap();
        let root = source.setup().roots[screen].root;
        source.change_window_attributes(root, &xproto::ChangeWindowAttributesAux::new().event_mask(xproto::EventMask::SUBSTRUCTURE_NOTIFY)).unwrap().check().unwrap();
        let owner = source.generate_id().unwrap();
        source.create_window(COPY_DEPTH_FROM_PARENT, owner, root, 20, 20, 20, 20, 0, xproto::WindowClass::INPUT_OUTPUT, 0, &xproto::CreateWindowAux::new()).unwrap();
        let atoms = Atoms::new(&source).unwrap();
        source.set_selection_owner(owner, atoms.selection, x11rb::CURRENT_TIME).unwrap();
        source.warp_pointer(NONE, root, 0, 0, 0, 0, 400, 300).unwrap();
        source.xtest_fake_input(xproto::BUTTON_PRESS_EVENT, 1, 0, root, 0, 0, 0).unwrap();
        source.xtest_fake_input(xproto::MOTION_NOTIFY_EVENT, 0, 0, root, 795, 300, 0).unwrap();
        source.flush().unwrap();
        assert_eq!(source.get_window_attributes(window).unwrap().reply().unwrap().map_state, xproto::MapState::VIEWABLE);
        let message = |kind, data| source.send_event(false, window, xproto::EventMask::NO_EVENT,
            xproto::ClientMessageEvent::new(32, window, kind, data)).unwrap();
        let nonowner = source.generate_id().unwrap();
        source.create_window(COPY_DEPTH_FROM_PARENT, nonowner, root, 20, 20, 20, 20, 0,
            xproto::WindowClass::INPUT_OUTPUT, 0, &xproto::CreateWindowAux::new()).unwrap();
        message(atoms.enter, [nonowner, 5 << 24, atoms.uri, 0, 0]);
        message(atoms.position, [nonowner, 0, (795 << 16) | 300, 0, atoms.copy]);
        message(atoms.drop, [nonowner, 0, 0, 0, 0]);
        source.flush().unwrap();
        let refused = event(&source, |event| matches!(event, Event::ClientMessage(message) if message.type_ == atoms.finished)).await;
        let Event::ClientMessage(refused) = refused else { unreachable!() };
        assert_eq!(refused.data.as_data32()[1], 0, "a non-owner cannot cause selection conversion");
        assert!(drops.try_recv().is_err());
        message(atoms.enter, [owner, 5 << 24, atoms.uri, 0, 0]);
        message(atoms.position, [owner, 0, (795 << 16) | 300, 0, atoms.copy]);
        source.flush().unwrap();
        let status = event(&source, |event| matches!(event, Event::ClientMessage(message) if message.type_ == atoms.status)).await;
        let Event::ClientMessage(status) = status else { unreachable!() };
        assert_eq!(status.data.as_data32()[1] & 1, 1);
        source.xtest_fake_input(xproto::BUTTON_RELEASE_EVENT, 1, 0, root, 0, 0, 0).unwrap();
        message(atoms.drop, [owner, 0, 0, 0, 0]);
        source.flush().unwrap();
        let request = event(&source, |event| matches!(event, Event::SelectionRequest(_))).await;
        let Event::SelectionRequest(request) = request else { unreachable!() };
        assert_eq!(request.selection, atoms.selection);
        source.change_property8(xproto::PropMode::REPLACE, request.requestor, request.property, atoms.uri,
            b"file:///tmp/hello%20world.txt\r\nfile:///tmp/another.txt\r\n").unwrap();
        source.send_event(false, request.requestor, xproto::EventMask::NO_EVENT, xproto::SelectionNotifyEvent {
            response_type: xproto::SELECTION_NOTIFY_EVENT, sequence: 0, time: request.time, requestor: request.requestor,
            selection: request.selection, target: request.target, property: request.property,
        }).unwrap();
        source.flush().unwrap();
        let drop = tokio::time::timeout(Duration::from_secs(5), drops.recv()).await.unwrap().unwrap();
        assert_eq!(drop.peer, "paired-id");
        assert_eq!(drop.paths, [PathBuf::from("/tmp/hello world.txt"), PathBuf::from("/tmp/another.txt")]);
        let finished = event(&source, |event| matches!(event, Event::ClientMessage(message) if message.type_ == atoms.finished)).await;
        let Event::ClientMessage(finished) = finished else { unreachable!() };
        assert_eq!(&finished.data.as_data32()[1..3], &[1, atoms.copy]);
        assert_eq!(source.get_window_attributes(window).unwrap().reply().unwrap().map_state, xproto::MapState::UNMAPPED);
        std::mem::drop(zones_tx);
        worker.await.unwrap().unwrap();
    }
}
