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

/// Watches the clipboard from before the sensitivity query until the content has
/// been read, so that the verdict and the content are of the same copy. They are
/// separate requests (the types the owner offers, then the data), and a copy made
/// between them would be read under the verdict of the one before: a password
/// taken for ordinary text. [`Guard::finish`] says if anything changed.
///
/// X11 (and XWayland, which is what GNOME's Wayland session uses) tells of every
/// change of the CLIPBOARD owner, also one by the same window, through XFixes: the
/// guard's own connection hears of it. The Wayland data-control protocol has no such
/// stream here, so the offered types are asked for again afterwards: a copy that
/// becomes sensitive in between offers the marker type, and shows.
#[cfg(target_os = "linux")]
pub(super) enum Guard {
    X11 { connection: x11rb::rust_connection::RustConnection, clipboard: u32 },
    Offers(std::collections::HashSet<String>),
    /// Nothing to watch with (no XFixes; types that can't be asked for).
    Unwatched,
}

#[cfg(target_os = "linux")]
fn offers() -> Option<std::collections::HashSet<String>> {
    use wl_clipboard_rs::paste::{get_mime_types, ClipboardType, Seat, Error};
    match get_mime_types(ClipboardType::Regular, Seat::Unspecified) {
        Ok(types) => Some(types),
        Err(Error::ClipboardEmpty | Error::NoSeats) => Some(Default::default()),
        Err(_) => None,
    }
}

#[cfg(target_os = "linux")]
impl Guard {
    pub(super) fn begin() -> anyhow::Result<Guard> {
        if super::wayland_data_control() {
            return Ok(offers().map_or(Guard::Unwatched, Guard::Offers));
        }
        use x11rb::{connection::Connection, protocol::{xfixes::{ConnectionExt as _, SelectionEventMask}, xproto::{ConnectionExt, CreateWindowAux, WindowClass}}};
        let (connection, screen) = x11rb::connect(None)?;
        let setup = || -> anyhow::Result<u32> {
            connection.xfixes_query_version(5, 0)?.reply()?;
            let root = connection.setup().roots[screen].root;
            let clipboard = connection.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
            let window = connection.generate_id()?;
            connection.create_window(0, window, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new())?.check()?;
            // `check` is a round trip: the server has taken the request before the caller goes on to ask anything.
            connection.xfixes_select_selection_input(window, clipboard,
                SelectionEventMask::SET_SELECTION_OWNER | SelectionEventMask::SELECTION_WINDOW_DESTROY | SelectionEventMask::SELECTION_CLIENT_CLOSE)?.check()?;
            Ok(clipboard)
        };
        match setup() {
            Ok(clipboard) => Ok(Guard::X11 { connection, clipboard }),
            Err(error) => {
                tracing::debug!("clipboard change guard unavailable: {error:#}");
                Ok(Guard::Unwatched)
            }
        }
    }

    /// Fails when the clipboard changed since [`Guard::begin`] (the caller reads again).
    pub(super) fn finish(self) -> anyhow::Result<()> {
        match self {
            Guard::Unwatched => Ok(()),
            Guard::Offers(before) => match offers() {
                Some(after) if after != before => anyhow::bail!("clipboard changed during read; retry"),
                _ => Ok(()),
            },
            Guard::X11 { connection, clipboard } => {
                use x11rb::{connection::Connection as _, protocol::{Event, xproto::ConnectionExt}};
                // Events come before the reply of a later request on the same connection: once
                // this returns, every change the server has made so far has been delivered.
                connection.get_input_focus()?.reply()?;
                while let Some(event) = connection.poll_for_event()? {
                    if matches!(event, Event::XfixesSelectionNotify(change) if change.selection == clipboard) {
                        anyhow::bail!("clipboard changed during read; retry");
                    }
                }
                Ok(())
            }
        }
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
    /// A copy made after the guard began is reported by `finish`, also by the same owner
    /// window (the owner id alone would not show it); no copy, no report.
    #[test]
    #[ignore = "requires an explicitly private X11 display and xclip"]
    fn real_x11_guard_reports_a_copy_made_while_it_watched() {
        use std::io::Write;
        assert_eq!(std::env::var("ONE_MK_PRIVATE_X11").as_deref(), Ok("1"));
        assert!(std::env::var_os("WAYLAND_DISPLAY").is_none());
        struct Owner(std::process::Child);
        impl Drop for Owner { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
        let copy = |text: &[u8]| {
            let mut owner = Owner(std::process::Command::new("xclip").args(["-selection", "clipboard", "-in", "-quiet"])
                .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap());
            owner.0.stdin.take().unwrap().write_all(text).unwrap();
            owner
        };
        let _first = copy(b"ordinary");
        std::thread::sleep(std::time::Duration::from_millis(300));
        let quiet = super::Guard::begin().unwrap();
        assert!(matches!(quiet, super::Guard::X11 { .. }));
        quiet.finish().expect("nothing was copied");
        let watching = super::Guard::begin().unwrap();
        let _second = copy(b"marked");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(watching.finish().is_err(), "a copy during the read went unnoticed");
    }

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
