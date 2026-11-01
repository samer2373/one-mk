use super::{Action, Permission, Shared, State, apple_permission};
use objc2::{AnyThread, DefinedClass, define_class, msg_send, rc::autoreleasepool, runtime::ProtocolObject};
use objc2_core_foundation::{CFRunLoop, CFRunLoopSource, CFRunLoopSourceContext, kCFRunLoopDefaultMode};
use objc2_core_location::{CLAuthorizationStatus, CLLocationManager, CLLocationManagerDelegate};
use objc2_core_wlan::{CWEventDelegate, CWEventType, CWWiFiClient};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};
use std::ffi::c_void;
use std::sync::{Arc, Mutex, mpsc};

struct Control {
    sender: mpsc::Sender<Action>,
    wake: Mutex<Option<(usize, usize)>>,
}

impl Control {
    fn send(&self, action: Action) {
        let wake = self.wake.lock().unwrap();
        let _ = self.sender.send(action);
        if let Some((source, run_loop)) = *wake {
            unsafe {
                (&*(source as *const CFRunLoopSource)).signal();
                (&*(run_loop as *const CFRunLoop)).wake_up();
            }
        }
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[ivars = Arc<Control>]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl CWEventDelegate for Delegate {
        #[unsafe(method(ssidDidChangeForWiFiInterfaceWithName:))]
        fn ssid_changed(&self, _name: &NSString) { self.ivars().send(Action::Refresh); }

        #[unsafe(method(linkDidChangeForWiFiInterfaceWithName:))]
        fn link_changed(&self, _name: &NSString) { self.ivars().send(Action::Refresh); }

        #[unsafe(method(powerStateDidChangeForWiFiInterfaceWithName:))]
        fn power_changed(&self, _name: &NSString) { self.ivars().send(Action::Refresh); }

        #[unsafe(method(clientConnectionInterrupted))]
        fn interrupted(&self) { self.ivars().send(Action::Refresh); }

        #[unsafe(method(clientConnectionInvalidated))]
        fn invalidated(&self) { self.ivars().send(Action::Refresh); }
    }

    unsafe impl CLLocationManagerDelegate for Delegate {
        #[unsafe(method(locationManagerDidChangeAuthorization:))]
        fn authorization_changed(&self, _manager: &CLLocationManager) { self.ivars().send(Action::Refresh); }

        #[unsafe(method(locationManager:didChangeAuthorizationStatus:))]
        fn authorization_changed_legacy(&self, _manager: &CLLocationManager, _status: CLAuthorizationStatus) {
            self.ivars().send(Action::Refresh);
        }
    }
);

struct Local {
    receiver: mpsc::Receiver<Action>,
    manager: objc2::rc::Retained<CLLocationManager>,
    client: objc2::rc::Retained<CWWiFiClient>,
    shared: Arc<Shared>,
    monitoring: bool,
}

impl Local {
    fn refresh(&mut self) {
        let mut state = State { permission: apple_permission(unsafe { self.manager.authorizationStatus() }.0), ..State::default() };
        unsafe {
            if !self.monitoring {
                let _ = self.client.stopMonitoringAllEventsAndReturnError();
                let registered = [CWEventType::SSIDDidChange, CWEventType::LinkDidChange, CWEventType::PowerDidChange]
                    .map(|event| self.client.startMonitoringEventWithType_error(event).is_ok());
                self.monitoring = registered.into_iter().all(|registered| registered);
            }
            state.unavailable = !self.monitoring;
            if let Some(interface) = self.client.interface() {
                if state.permission == Permission::Allowed
                    && let Some(ssid) = interface.ssid().filter(|ssid| !ssid.is_empty())
                {
                    state.ssids.push(ssid.to_string());
                }
            } else {
                state.unavailable = true;
            }
        }
        self.shared.publish(state);
    }

    fn permission(&self) {
        match apple_permission(unsafe { self.manager.authorizationStatus() }.0) {
            Permission::NotDetermined => unsafe { self.manager.requestAlwaysAuthorization(); },
            _ => {
                let _ = std::process::Command::new("/usr/bin/open")
                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_LocationServices").spawn();
            }
        }
    }
}

unsafe extern "C-unwind" fn perform(info: *mut c_void) {
    autoreleasepool(|_| {
        let local = unsafe { &mut *info.cast::<Local>() };
        while let Ok(action) = local.receiver.try_recv() {
            match action {
                Action::Refresh => local.refresh(),
                Action::Permission => local.permission(),
                Action::Stop => {
                    if let Some(run_loop) = CFRunLoop::current() { run_loop.stop(); }
                    return;
                }
            }
        }
    });
}

pub(super) fn start(shared: Arc<Shared>) -> Box<dyn Fn(Action) + Send + Sync> {
    let (sender, receiver) = mpsc::channel();
    let control = Arc::new(Control { sender, wake: Mutex::new(None) });
    let worker_control = control.clone();
    std::thread::spawn(move || autoreleasepool(|_| unsafe {
        let Some(run_loop) = CFRunLoop::current() else {
            shared.publish(State { unavailable: true, ..State::default() });
            return;
        };
        let manager = CLLocationManager::new();
        let client = CWWiFiClient::sharedWiFiClient();
        let delegate = Delegate::alloc().set_ivars(worker_control.clone());
        let delegate: objc2::rc::Retained<Delegate> = msg_send![super(delegate), init];
        manager.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        client.setDelegate(Some(&delegate));
        let mut local = Box::new(Local { receiver, manager, client, shared, monitoring: false });
        let mut context = CFRunLoopSourceContext {
            version: 0, info: (&mut *local as *mut Local).cast(), retain: None, release: None,
            copyDescription: None, equal: None, hash: None, schedule: None, cancel: None, perform: Some(perform),
        };
        let source = CFRunLoopSource::new(None, 0, &mut context).unwrap();
        run_loop.add_source(Some(&source), kCFRunLoopDefaultMode);
        *worker_control.wake.lock().unwrap() = Some(((&*source as *const CFRunLoopSource) as usize, (&*run_loop as *const CFRunLoop) as usize));
        local.refresh();
        source.signal();
        CFRunLoop::run();
        *worker_control.wake.lock().unwrap() = None;
        source.invalidate();
        let _ = local.client.stopMonitoringAllEventsAndReturnError();
        local.client.setDelegate(None);
        local.manager.setDelegate(None);
    }));
    Box::new(move |action| control.send(action))
}
