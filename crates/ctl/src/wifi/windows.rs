use super::{Action, Permission, Shared, State, windows_permission};
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::{Arc, mpsc};
use windows::Foundation::TypedEventHandler;
use windows::Security::Authorization::AppCapabilityAccess::AppCapability;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows::core::HSTRING;
use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_SUCCESS, HANDLE};
use windows_sys::Win32::NetworkManagement::WiFi::*;

pub(super) fn start(shared: Arc<Shared>) -> Box<dyn Fn(Action) + Send + Sync> {
    let (sender, receiver) = mpsc::channel();
    let notifications = sender.clone();
    std::thread::spawn(move || unsafe {
        let initialized = RoInitialize(RO_INIT_MULTITHREADED).is_ok();
        let capability = AppCapability::Create(&HSTRING::from("wiFiControl")).ok();
        let permission_events = notifications.clone();
        let token = capability.as_ref().and_then(|capability| capability.AccessChanged(&TypedEventHandler::new(move |_, _| {
            let _ = permission_events.send(Action::Refresh);
            Ok(())
        })).ok());
        let context = Box::new(notifications);
        let mut handle = null_mut();
        let mut registered = false;
        let refresh = |handle: &mut HANDLE, registered: &mut bool| {
            if handle.is_null() {
                let mut version = 0;
                let opened = WlanOpenHandle(2, null(), &mut version, handle);
                if opened != ERROR_SUCCESS {
                    shared.publish(State { permission: windows_permission(opened), unavailable: opened != ERROR_ACCESS_DENIED, ..State::default() });
                    return;
                }
            }
            let registration = if *registered { ERROR_SUCCESS } else {
                WlanRegisterNotification(*handle, WLAN_NOTIFICATION_SOURCE_ACM, 1, Some(changed),
                    (&*context as *const mpsc::Sender<Action>).cast(), null(), null_mut())
            };
            *registered = registration == ERROR_SUCCESS;
            let mut state = current(*handle);
            if registration == ERROR_ACCESS_DENIED {
                state.permission = Permission::Denied;
            } else if registration != ERROR_SUCCESS {
                state.unavailable = true;
            }
            shared.publish(state);
        };
        refresh(&mut handle, &mut registered);
        while let Ok(action) = receiver.recv() {
            match action {
                Action::Stop => break,
                Action::Refresh => refresh(&mut handle, &mut registered),
                Action::Permission => {
                    let _ = std::process::Command::new("explorer.exe").arg("ms-settings:privacy-location").spawn();
                }
            }
        }
        if !handle.is_null() {
            WlanRegisterNotification(handle, WLAN_NOTIFICATION_SOURCE_NONE, 1, None, null(), null(), null_mut());
            WlanCloseHandle(handle, null());
        }
        drop(context);
        if let (Some(capability), Some(token)) = (capability, token) {
            let _ = capability.RemoveAccessChanged(token);
        }
        if initialized { RoUninitialize(); }
    });
    Box::new(move |action| { let _ = sender.send(action); })
}

unsafe extern "system" fn changed(data: *mut L2_NOTIFICATION_DATA, context: *mut c_void) {
    if data.is_null() || context.is_null() { return; }
    let data = unsafe { &*data };
    if data.NotificationSource != WLAN_NOTIFICATION_SOURCE_ACM { return; }
    if [wlan_notification_acm_connection_complete, wlan_notification_acm_disconnected,
        wlan_notification_acm_interface_arrival, wlan_notification_acm_interface_removal,
        wlan_notification_acm_operational_state_change].contains(&(data.NotificationCode as i32))
    {
        let sender = unsafe { &*context.cast::<mpsc::Sender<Action>>() };
        let _ = sender.send(Action::Refresh);
    }
}

unsafe fn current(handle: HANDLE) -> State {
    let mut state = State { permission: Permission::Allowed, ..State::default() };
    let mut list = null_mut();
    let result = unsafe { WlanEnumInterfaces(handle, null(), &mut list) };
    if result != ERROR_SUCCESS {
        return State { permission: windows_permission(result), unavailable: result != ERROR_ACCESS_DENIED, ..State::default() };
    }
    if list.is_null() { return State { unavailable: true, ..state }; }
    let interfaces = unsafe { std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), (*list).dwNumberOfItems as usize) };
    for interface in interfaces {
        if interface.isState != wlan_interface_state_connected { continue; }
        let mut size = 0;
        let mut data = null_mut();
        let result = unsafe { WlanQueryInterface(handle, &interface.InterfaceGuid, wlan_intf_opcode_current_connection,
            null(), &mut size, &mut data, null_mut()) };
        if result == ERROR_ACCESS_DENIED {
            state.permission = Permission::Denied;
        } else if result != ERROR_SUCCESS {
            state.unavailable = true;
        } else if !data.is_null() && size as usize >= size_of::<WLAN_CONNECTION_ATTRIBUTES>() {
            let attributes = unsafe { &*data.cast::<WLAN_CONNECTION_ATTRIBUTES>() };
            let ssid = &attributes.wlanAssociationAttributes.dot11Ssid;
            if attributes.isState == wlan_interface_state_connected && ssid.uSSIDLength > 0 && ssid.uSSIDLength <= 32 {
                state.ssids.push(String::from_utf8_lossy(&ssid.ucSSID[..ssid.uSSIDLength as usize]).into_owned());
            }
        } else {
            state.unavailable = true;
        }
        if !data.is_null() { unsafe { WlanFreeMemory(data); } }
    }
    unsafe { WlanFreeMemory(list.cast()); }
    state
}
