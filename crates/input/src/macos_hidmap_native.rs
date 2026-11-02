use crate::macos_hidmap::{Lifecycle, Mapping, Runner};
use crate::{Bus, Fingerprint};
use anyhow::{Context, Result, bail};
use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use std::ffi::c_void;
use std::ptr::NonNull;
use objc2_core_graphics::{CGEvent, CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation};

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDEventSystemClientCreateSimpleClient(allocator: *const c_void) -> *mut CFType;
    fn IOHIDEventSystemClientCopyServices(client: &CFType) -> *mut CFArray<CFType>;
    fn IOHIDServiceClientGetRegistryID(service: &CFType) -> *const CFType;
    fn IOHIDServiceClientConformsTo(service: &CFType, page: u32, usage: u32) -> u8;
    fn IOHIDServiceClientCopyProperty(service: &CFType, key: &CFString) -> *mut CFType;
    fn IOHIDServiceClientSetProperty(service: &CFType, key: &CFString, value: &CFType) -> u8;
}

pub(crate) struct Completion {
    pub result: std::result::Result<Option<crate::macos_hidmap::Plan>, String>,
    pub restored: Vec<Mapping>,
}

pub(crate) struct Native {
    requests: Option<std::sync::mpsc::Sender<(u64, Option<crate::macos_hidmap::Plan>)>>,
    thread: Option<std::thread::JoinHandle<()>>,
    replies: std::sync::mpsc::Receiver<(u64, Completion)>,
    serial: u64,
    pending: bool,
    desired: Option<crate::macos_hidmap::Plan>,
    pub error: Option<String>,
}

impl Native {
    pub(crate) fn start(dir: std::path::PathBuf, inbox: std::sync::Arc<crate::macos::Inbox>) -> std::io::Result<Self> {
        let (requests, receiver) = std::sync::mpsc::channel::<(u64, Option<crate::macos_hidmap::Plan>)>();
        let (sender, replies) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new().name("one-mk-keyboard-mappings".into()).spawn(move || {
            let recovered = System::new().and_then(|runner| Lifecycle::recover(runner, &dir));
            let status = recovered.as_ref().map(|_| None).map_err(|error| format!("Keyboard recovery unavailable: {error:#}. Sharing still works; local rules need recovery before restarting oneMK."));
            let _ = sender.send((0, Completion { result: status, restored: Vec::new() }));
            inbox.signal();
            let mut recovered = recovered.map_err(|error| format!("Keyboard recovery unavailable: {error:#}. Sharing still works; local rules need recovery before restarting oneMK."));
            while let Ok((serial, plan)) = receiver.recv() {
                let restored = recovered.as_ref().map_or_else(|_| Vec::new(), |lifecycle| lifecycle.owned_mappings());
                let result = match recovered.as_mut() {
                    Ok(lifecycle) => match &plan { Some(plan) => lifecycle.apply(plan), None => lifecycle.restore() }
                        .map(|()| plan).map_err(|error| format!("{error:#}")),
                    Err(error) => Err(error.clone()),
                };
                let _ = sender.send((serial, Completion { result, restored }));
                inbox.signal();
            }
            if let Ok(lifecycle) = &mut recovered && let Err(error) = lifecycle.restore() {
                tracing::error!(%error, "keyboard recovery record kept for the next start");
            }
        })?;
        Ok(Self { requests: Some(requests), thread: Some(thread), replies, serial: 0, pending: true, desired: None, error: None })
    }

    pub(crate) fn request(&mut self, plan: Option<crate::macos_hidmap::Plan>) -> bool {
        if self.desired == plan && self.error.is_none() { return !self.pending; }
        self.desired = plan.clone();
        self.serial += 1;
        self.pending = true;
        if self.requests.as_ref().is_none_or(|requests| requests.send((self.serial, plan)).is_err()) {
            self.pending = false;
            self.error.get_or_insert_with(|| "Keyboard mapping worker unavailable; sharing still works".into());
        }
        !self.pending
    }

    pub(crate) fn poll(&mut self) -> Option<Completion> {
        let mut latest = None;
        while let Ok((serial, completion)) = self.replies.try_recv() {
            if serial == self.serial {
                self.pending = false;
                self.error = completion.result.as_ref().err().cloned();
                latest = Some(completion);
            }
        }
        latest
    }

    pub(crate) fn pending(&self) -> bool { self.pending }
}

impl Drop for Native {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

pub(crate) struct System {
    client: CFRetained<CFType>,
    services: Vec<(u64, CFRetained<CFType>)>,
}

impl System {
    pub(crate) fn new() -> Result<Self> {
        let pointer = NonNull::new(unsafe { IOHIDEventSystemClientCreateSimpleClient(std::ptr::null()) })
            .context("macOS could not open its keyboard mapping service")?;
        let mut system = Self { client: unsafe { CFRetained::from_raw(pointer) }, services: Vec::new() };
        system.refresh_services()?;
        Ok(system)
    }

    fn services(&self) -> Result<CFRetained<CFArray<CFType>>> {
        let pointer = NonNull::new(unsafe { IOHIDEventSystemClientCopyServices(&self.client) })
            .context("macOS could not list keyboard mapping services")?;
        Ok(unsafe { CFRetained::from_raw(pointer) })
    }

    fn refresh_services(&mut self) -> Result<()> {
        self.services = self.services()?.iter().filter_map(|service| {
            let value = unsafe { IOHIDServiceClientGetRegistryID(&service).as_ref() }?;
            let registry_id = value.downcast_ref::<CFNumber>()?.as_i64()? as u64;
            Some((registry_id, service))
        }).collect();
        Ok(())
    }

    fn service(&self, registry_id: u64) -> Option<CFRetained<CFType>> {
        self.services.iter().find(|(identifier, _)| *identifier == registry_id).map(|(_, service)| service.clone())
    }
}

fn property(service: &CFType, key: &'static str) -> Option<CFRetained<CFType>> {
    NonNull::new(unsafe { IOHIDServiceClientCopyProperty(service, &CFString::from_static_str(key)) })
        .map(|pointer| unsafe { CFRetained::from_raw(pointer) })
}

fn number(service: &CFType, key: &'static str) -> Option<i64> {
    property(service, key)?.downcast_ref::<CFNumber>()?.as_i64()
}

fn string(service: &CFType, key: &'static str) -> Option<String> {
    Some(property(service, key)?.downcast_ref::<CFString>()?.to_string())
}

impl Runner for System {
    fn keyboards(&mut self) -> Result<Vec<(u64, Fingerprint)>> {
        let mut keyboards = Vec::new();
        self.refresh_services()?;
        for (registry_id, service) in &self.services {
            if unsafe { IOHIDServiceClientConformsTo(service, 1, 6) } == 0 { continue; }
            let vendor = number(service, "VendorID").and_then(|value| u16::try_from(value).ok()).unwrap_or(0);
            let product = number(service, "ProductID").and_then(|value| u16::try_from(value).ok()).unwrap_or(0);
            let serial = string(service, "SerialNumber");
            let name = string(service, "Product").unwrap_or_else(|| "Keyboard".into());
            let transport = string(service, "Transport").unwrap_or_default().to_ascii_lowercase();
            let builtin = property(service, "Built-In").and_then(|value| value.downcast_ref::<CFBoolean>().map(|value| value.as_bool())).unwrap_or(false);
            let bus = if builtin || transport == "spi" || transport == "fifo" { Bus::Internal }
                else if transport.starts_with("bluetooth") { Bus::Bluetooth }
                else if transport == "usb" { Bus::Usb } else { Bus::Unknown };
            keyboards.push((*registry_id, Fingerprint::new(bus, vendor, product, serial.as_deref(), &name)));
        }
        Ok(keyboards)
    }

    fn read(&mut self, registry_id: u64) -> Result<Option<Vec<Mapping>>> {
        let Some(service) = self.service(registry_id) else { return Ok(None) };
        let Some(value) = property(&service, "UserKeyMapping") else { return Ok(Some(Vec::new())) };
        let array = value.downcast_ref::<CFArray>().context("macOS returned an unreadable keyboard mapping; left unchanged")?;
        let array = unsafe { array.cast_unchecked::<CFType>() };
        let mut mappings = Vec::new();
        for value in array.iter() {
            let entry = value.downcast_ref::<CFDictionary>().context("Unrecognized keyboard mapping; left unchanged")?;
            let entry = unsafe { entry.cast_unchecked::<CFType, CFType>() };
            if entry.len() != 2 { bail!("Unrecognized keyboard mapping fields; left unchanged"); }
            let integer = |key| -> Result<u64> {
                let key = CFString::from_static_str(key);
                let value = entry.get(key.as_ref()).context("Missing keyboard mapping field")?;
                let number = value.downcast_ref::<CFNumber>().and_then(|value| value.as_i64()).context("Invalid keyboard mapping field")?;
                u64::try_from(number).context("Invalid keyboard mapping usage")
            };
            mappings.push(Mapping { source: integer("HIDKeyboardModifierMappingSrc")?, destination: integer("HIDKeyboardModifierMappingDst")? });
        }
        Ok(Some(mappings))
    }

    fn write(&mut self, registry_id: u64, expected: &[Mapping], mappings: &[Mapping]) -> Result<()> {
        let service = self.service(registry_id).context("The keyboard disconnected before its mapping could be changed")?;
        let previous = self.read(registry_id)?.unwrap_or_default();
        if previous != expected { bail!("Another keyboard tool changed this keyboard; its mappings were left unchanged"); }
        let changed = previous.iter().filter(|entry| !mappings.contains(entry)).map(|entry| entry.destination)
            .chain(mappings.iter().filter(|entry| !previous.contains(entry)).map(|entry| entry.source));
        let mut released = Vec::new();
        for usage in changed {
            if released.contains(&usage) { continue; }
            released.push(usage);
            let Some(code) = crate::devcorr::hid_key_to_evdev((usage & 0xffff) as u32) else { continue };
            if code == 58 { continue; }
            let Some(keycode) = crate::macos_logic::evdev_to_mac(code) else { continue };
            if !CGEventSource::key_state(CGEventSourceStateID::CombinedSessionState, keycode) { continue; }
            let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).context("Could not release the old keyboard mapping")?;
            CGEventSource::set_local_events_suppression_interval(Some(&source), 0.0);
            let event = CGEvent::new_keyboard_event(Some(&source), keycode, false).context("Could not release the old keyboard mapping")?;
            let mut flags = crate::macos_localmap_logic::Flags::default();
            flags.key(code, true, true);
            CGEvent::set_flags(Some(&event), CGEventFlags(flags.apply(CGEventSource::flags_state(CGEventSourceStateID::CombinedSessionState).0, 0)));
            CGEvent::set_integer_value_field(Some(&event), CGEventField::EventSourceUserData, crate::macos::OUR_EVENTS);
            CGEvent::post(CGEventTapLocation::SessionEventTap, Some(&event));
        }
        let source = CFString::from_static_str("HIDKeyboardModifierMappingSrc");
        let destination = CFString::from_static_str("HIDKeyboardModifierMappingDst");
        let entries: Vec<_> = mappings.iter().map(|mapping| {
            CFDictionary::from_slices(&[&*source, &*destination], &[&*CFNumber::new_i64(mapping.source as i64), &*CFNumber::new_i64(mapping.destination as i64)])
        }).collect();
        let array = CFArray::from_objects(&entries.iter().map(|entry| &**entry).collect::<Vec<_>>());
        if unsafe { IOHIDServiceClientSetProperty(&service, &CFString::from_static_str("UserKeyMapping"), array.as_ref()) } == 0 {
            bail!("macOS refused to change the keyboard mapping");
        }
        Ok(())
    }
}
