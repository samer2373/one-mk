use super::{Drop, Zone, MAX_URI_BYTES, uri_list};
use anyhow::{Result, bail};
use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchTime};
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_app_kit::*;
use objc2_foundation::{MainThreadMarker, NSArray, NSObjectProtocol, NSPoint, NSRect, NSSize, NSURL};
use one_mk_proto::{Pos, drag::{Drag, Change}};
use std::{cell::RefCell, ptr::NonNull, sync::atomic::{AtomicBool, Ordering}, time::Duration};
use tokio::sync::{mpsc, watch};

static RUNNING: AtomicBool = AtomicBool::new(false);
thread_local! { static UI: RefCell<Option<Ui>> = const { RefCell::new(None) }; }

struct ViewState { zone: RefCell<Option<Zone>>, dropped: mpsc::Sender<Drop> }

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ViewState]
    struct FileView;

    unsafe impl NSObjectProtocol for FileView {}
    unsafe impl NSDraggingDestination for FileView {
        #[unsafe(method(draggingEntered:))]
        fn entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            if self.ivars().zone.borrow().is_some() && sender.draggingSourceOperationMask().contains(NSDragOperation::Copy) {
                NSDragOperation::Copy
            } else { NSDragOperation::None }
        }

        #[unsafe(method(draggingUpdated:))]
        fn updated(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation { self.operation(sender) }

        #[unsafe(method(prepareForDragOperation:))]
        fn prepare(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool { self.operation(sender) == NSDragOperation::Copy }

        #[unsafe(method(draggingExited:))]
        fn exited(&self, _: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            self.ivars().zone.replace(None);
            DispatchQueue::main().exec_async(|| with_ui(Ui::cancel));
        }

        #[unsafe(method(performDragOperation:))]
        fn perform(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            self.accept_drop(sender)
        }
    }
);

impl FileView {
    fn operation(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
        if self.ivars().zone.borrow().is_some() && sender.draggingSourceOperationMask().contains(NSDragOperation::Copy) {
            NSDragOperation::Copy
        } else { NSDragOperation::None }
    }

    fn accept_drop(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
        if self.operation(sender) != NSDragOperation::Copy { return false; }
        let Some(zone) = self.ivars().zone.take() else { return false };
        let accepted = dropped_paths(sender).ok().is_some_and(|paths|
            self.ivars().dropped.try_send(Drop { peer: zone.peer, paths }).is_ok());
        DispatchQueue::main().exec_async(|| with_ui(Ui::cancel));
        accepted
    }
}

fn dropped_paths(sender: &ProtocolObject<dyn NSDraggingInfo>) -> Result<Vec<std::path::PathBuf>> {
    let pasteboard = sender.draggingPasteboard();
    let items = pasteboard.pasteboardItems().ok_or_else(|| anyhow::anyhow!("drop contains no file URLs"))?;
    if items.is_empty() || items.len() > 10_000 { bail!("drop contains too many items"); }
    let mut text = String::new();
    for item in items.iter() {
        let value = item.stringForType(unsafe { NSPasteboardTypeFileURL }).ok_or_else(|| anyhow::anyhow!("drop contains a non-file item"))?;
        if value.len() > MAX_URI_BYTES || text.len() + value.len() * 4 > MAX_URI_BYTES { bail!("drop URLs are too large"); }
        let url = NSURL::URLWithString(&value).and_then(|url| url.filePathURL())
            .and_then(|url| url.absoluteString()).ok_or_else(|| anyhow::anyhow!("cannot resolve file URL"))?;
        if text.len() + url.len() * 4 > MAX_URI_BYTES { bail!("drop URLs are too large"); }
        text.push_str(&url.to_string());
        text.push('\n');
    }
    uri_list(text.as_bytes())
}

struct Ui {
    window: Retained<NSWindow>,
    view: Retained<FileView>,
    monitors: Vec<Retained<AnyObject>>,
    zones: Vec<Zone>,
    state: Drag,
    held: bool,
    generation: u64,
}

fn with_ui(action: impl FnOnce(&mut Ui)) {
    UI.with(|ui| { if let Some(ui) = ui.borrow_mut().as_mut() { action(ui); } });
}

impl Ui {
    fn new(mtm: MainThreadMarker, dropped: mpsc::Sender<Drop>) -> Result<Self> {
        let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(1., 1.));
        let view = FileView::alloc(mtm).set_ivars(ViewState { zone: RefCell::new(None), dropped });
        let view: Retained<FileView> = unsafe { msg_send![super(view), initWithFrame: frame] };
        view.registerForDraggedTypes(&NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]));
        let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), frame,
            NSWindowStyleMask::Borderless, NSBackingStoreType::Buffered, false) };
        unsafe { window.setReleasedWhenClosed(false); }
        window.setContentView(Some(&view));
        window.setLevel(NSFloatingWindowLevel);
        window.setBackgroundColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(0.15, 0.45, 0.75, 1.)));
        let global = RcBlock::new(|event: NonNull<NSEvent>| with_ui(|ui| ui.event(unsafe { event.as_ref() })));
        let mask = NSEventMask::LeftMouseDown | NSEventMask::LeftMouseUp | NSEventMask::LeftMouseDragged | NSEventMask::KeyDown;
        let global = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &global)
            .ok_or_else(|| anyhow::anyhow!("global drag observation unavailable; check Input Monitoring permissions"))?;
        let local = RcBlock::new(|event: NonNull<NSEvent>| {
            with_ui(|ui| ui.event(unsafe { event.as_ref() }));
            event.as_ptr()
        });
        let local = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &local) };
        let mut monitors = vec![global];
        monitors.extend(local);
        Ok(Self { window, view, monitors, zones: vec![], state: Drag::default(), held: false, generation: 0 })
    }

    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.state.cancel();
        self.view.ivars().zone.replace(None);
        self.window.orderOut(None);
    }

    fn event(&mut self, event: &NSEvent) {
        if event.r#type() == NSEventType::KeyDown {
            if event.keyCode() == 53 { self.cancel(); }
            return;
        }
        if event.r#type() == NSEventType::LeftMouseDown {
            self.generation = self.generation.wrapping_add(1);
            if self.state.pointer(false, None) == Change::Hide { self.cancel(); }
            self.held = true;
        } else if event.r#type() == NSEventType::LeftMouseUp {
            self.held = false;
            if self.view.ivars().zone.borrow().is_some() {
                let generation = self.generation;
                let _ = DispatchQueue::main().after(DispatchTime::try_from(Duration::from_millis(300)).expect("bounded delay"), move || {
                    with_ui(|ui| { if !ui.held && ui.generation == generation { ui.cancel(); ui.state.pointer(false, None); } });
                });
            }
            return;
        }
        if !self.held { return; }
        let mtm = self.window.mtm();
        let Some(screen) = NSScreen::screens(mtm).firstObject() else { self.cancel(); return };
        let height = screen.frame().size.height;
        let point = NSEvent::mouseLocation();
        let point = Pos::new(point.x, height - point.y);
        let zone = self.zones.iter().position(|zone| zone.rect.contains(point));
        match self.state.pointer(true, zone) {
            Change::Show(index) => {
                self.generation = self.generation.wrapping_add(1);
                let generation = self.generation;
                let _ = DispatchQueue::main().after(DispatchTime::try_from(Duration::from_secs(30)).expect("bounded delay"), move || {
                    with_ui(|ui| { if ui.generation == generation { ui.cancel(); } });
                });
                let zone = self.zones[index].clone();
                let rect = zone.rect;
                self.view.ivars().zone.replace(Some(zone));
                self.window.setFrame_display(NSRect::new(NSPoint::new(rect.x.into(), height - f64::from(rect.y + rect.h)),
                    NSSize::new(rect.w.into(), rect.h.into())), true);
                self.window.orderFrontRegardless();
            }
            Change::Hide => self.cancel(),
            Change::None => {}
        }
    }
}

impl std::ops::Drop for Ui {
    fn drop(&mut self) {
        self.cancel();
        for monitor in &self.monitors { unsafe { NSEvent::removeMonitor(monitor); } }
    }
}

pub(super) fn start(mut zones: watch::Receiver<Vec<Zone>>, dropped: mpsc::Sender<Drop>) -> Result<()> {
    if !RUNNING.load(Ordering::Acquire) { bail!("AppKit main-thread loop is not running"); }
    let (ready, initialized) = std::sync::mpsc::sync_channel(1);
    DispatchQueue::main().exec_async(move || {
        let result = MainThreadMarker::new().ok_or_else(|| anyhow::anyhow!("AppKit requires the main thread"))
            .and_then(|mtm| Ui::new(mtm, dropped));
        match result {
            Ok(ui) => { UI.with(|slot| slot.replace(Some(ui))); let _ = ready.send(Ok(())); }
            Err(error) => { let _ = ready.send(Err(error.to_string())); }
        }
    });
    initialized.recv()?.map_err(anyhow::Error::msg)?;
    let initial = zones.borrow_and_update().clone();
    DispatchQueue::main().exec_async(move || with_ui(|ui| ui.zones = initial));
    tokio::spawn(async move {
        while zones.changed().await.is_ok() {
            let zones = zones.borrow_and_update().clone();
            DispatchQueue::main().exec_async(move || with_ui(|ui| { ui.cancel(); ui.zones = zones; }));
        }
        DispatchQueue::main().exec_async(|| UI.with(|slot| { slot.take(); }));
    });
    Ok(())
}

pub fn with_main_loop(work: impl FnOnce() -> Result<()> + Send + 'static) -> Result<()> {
    let mtm = MainThreadMarker::new().ok_or_else(|| anyhow::anyhow!("AppKit requires the main thread"))?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    RUNNING.store(true, Ordering::Release);
    let worker = std::thread::Builder::new().name("one-mk-daemon".into()).spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
        DispatchQueue::main().exec_async(|| {
            UI.with(|slot| { slot.take(); });
            let app = NSApplication::sharedApplication(MainThreadMarker::new().expect("main queue"));
            app.stop(None);
            if let Some(event) = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
                NSEventType::ApplicationDefined, NSPoint::new(0., 0.), NSEventModifierFlags::empty(), 0., 0, None, 0, 0, 0)
            { app.postEvent_atStart(&event, true); }
        });
        result.unwrap_or_else(|_| Err(anyhow::anyhow!("daemon thread panicked")))
    })?;
    app.run();
    RUNNING.store(false, Ordering::Release);
    worker.join().map_err(|_| anyhow::anyhow!("daemon thread panicked"))?
}
