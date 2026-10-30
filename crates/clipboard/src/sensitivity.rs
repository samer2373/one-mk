#[cfg(target_os = "linux")]
pub(super) fn concealed(apps: &[String]) -> anyhow::Result<bool> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        use wl_clipboard_rs::paste::{get_mime_types, ClipboardType, Seat, Error};
        match get_mime_types(ClipboardType::Regular, Seat::Unspecified) {
            Ok(types) => return Ok(types.contains("x-kde-passwordManagerHint")),
            Err(Error::ClipboardEmpty | Error::NoSeats) => return Ok(false),
            Err(_) => {},
        }
    }
    use std::os::fd::AsRawFd;
    use x11rb::{connection::Connection, protocol::{Event, xproto::{ConnectionExt, AtomEnum, CreateWindowAux, WindowClass}}};
    let (connection, screen) = x11rb::connect(None)?;
    let root = connection.setup().roots[screen].root;
    let clipboard = connection.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
    let active = connection.intern_atom(false, b"_NET_ACTIVE_WINDOW")?.reply()?.atom;
    if super::history_watch::focused_apps(&connection, root, active).iter().any(|name| apps.iter().any(|app| name.eq_ignore_ascii_case(app))) { return Ok(true); }
    if connection.get_selection_owner(clipboard)?.reply()?.owner == 0 { return Ok(false); }
    let targets = connection.intern_atom(false, b"TARGETS")?.reply()?.atom;
    let hint = connection.intern_atom(false, b"x-kde-passwordManagerHint")?.reply()?.atom;
    let property = connection.intern_atom(false, b"ONE_MK_SENSITIVITY")?.reply()?.atom;
    let window = connection.generate_id()?;
    connection.create_window(0, window, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new())?.check()?;
    connection.convert_selection(window, clipboard, targets, property, x11rb::CURRENT_TIME)?;
    connection.flush()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(150);
    loop {
        while let Some(event) = connection.poll_for_event()? {
            if let Event::SelectionNotify(event) = event {
                if event.property == 0 { return Ok(false); }
                let result = connection.get_property(false, window, property, AtomEnum::ATOM, 0, 4096)?.reply()?;
                return Ok(result.value32().is_some_and(|mut atoms| atoms.any(|atom| atom == hint)));
            }
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() { anyhow::bail!("clipboard sensitivity query timed out"); }
        let mut descriptor = libc::pollfd { fd: connection.stream().as_raw_fd(), events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut descriptor, 1, remaining.as_millis().max(1) as i32) } < 0 { return Err(std::io::Error::last_os_error().into()); }
    }
}

#[cfg(windows)]
pub(super) fn generation() -> u64 {
    unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() as u64 }
}

#[cfg(target_os = "macos")]
pub(super) fn generation() -> u64 {
    objc2::rc::autoreleasepool(|_| objc2_app_kit::NSPasteboard::generalPasteboard().changeCount() as u64)
}

#[cfg(windows)]
pub(super) fn concealed(_: &[String]) -> anyhow::Result<bool> {
    use windows_sys::Win32::System::{DataExchange::*, Memory::{GlobalLock, GlobalUnlock, GlobalSize}};
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 { anyhow::bail!("clipboard sensitivity query unavailable"); }
        struct Open;
        impl Drop for Open { fn drop(&mut self) { unsafe { CloseClipboard(); } } }
        let _open = Open;
        let format = |name: &str| RegisterClipboardFormatW(name.encode_utf16().chain(Some(0)).collect::<Vec<_>>().as_ptr());
        let exclude = format("ExcludeClipboardContentFromMonitorProcessing");
        let history = format("CanIncludeInClipboardHistory");
        if exclude == 0 || history == 0 { anyhow::bail!("clipboard sensitivity formats unavailable"); }
        if IsClipboardFormatAvailable(exclude) != 0 { return Ok(true); }
        if IsClipboardFormatAvailable(history) == 0 { return Ok(false); }
        let handle = GetClipboardData(history);
        if handle.is_null() || GlobalSize(handle) < 4 { return Ok(true); }
        let data = GlobalLock(handle);
        if data.is_null() { return Ok(true); }
        let permitted = std::ptr::read_unaligned(data.cast::<u32>()) != 0;
        GlobalUnlock(handle);
        Ok(!permitted)
    }
}

#[cfg(target_os = "macos")]
pub(super) fn concealed(_: &[String]) -> anyhow::Result<bool> {
    objc2::rc::autoreleasepool(|_| {
        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        Ok(pasteboard.types().is_some_and(|types| types.iter().any(|kind| kind.to_string() == "org.nspasteboard.ConcealedType")))
    })
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
pub(super) fn concealed(_: &[String]) -> anyhow::Result<bool> { Ok(false) }

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    #[ignore = "requires an explicitly private X11 display and xclip"]
    fn real_x11_password_manager_target_is_concealed() {
        use std::io::Write;
        assert_eq!(std::env::var("ONE_MK_PRIVATE_X11").as_deref(), Ok("1"));
        assert!(std::env::var_os("WAYLAND_DISPLAY").is_none());
        struct Owner(std::process::Child);
        impl Drop for Owner { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
        let previous = super::owner().unwrap();
        let mut owner = Owner(std::process::Command::new("xclip").args(["-selection", "clipboard", "-in", "-quiet", "-target", "x-kde-passwordManagerHint"])
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap());
        owner.0.stdin.take().unwrap().write_all(b"dummy secret hint").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while super::owner().unwrap() == previous && std::time::Instant::now() < deadline { std::thread::sleep(std::time::Duration::from_millis(10)); }
        assert!(super::concealed(&[]).unwrap());
    }
}
#[cfg(target_os = "linux")]
pub(super) fn owner() -> anyhow::Result<u32> {
    use x11rb::protocol::xproto::ConnectionExt;
    let (connection, _) = x11rb::connect(None)?;
    let clipboard = connection.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
    Ok(connection.get_selection_owner(clipboard)?.reply()?.owner)
}
