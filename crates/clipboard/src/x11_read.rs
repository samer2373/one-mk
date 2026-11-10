//! Reading text and HTML from the X11 clipboard, INCR transfers included.
//!
//! arboard (3.7) gives up on an incremental (INCR) transfer when the next
//! chunk takes more than 10 ms, which a separate owner process often does
//! (xclip; XWayland bridging a Wayland app). It then asked for the next
//! format on the same window and destroyed that window mid-transfer, so the
//! owner waited for it forever (xclip stopped answering anyone, copying in
//! that app hung), and the next format read picked up the stale chunks:
//! about 1 MB or more of plain text came out as "text and HTML" (xclip
//! answers any format, and arboard checks the format only when the answer
//! is not incremental). Here each
//! format is read on its own fresh window, a chunk may take up to
//! [`CHUNK_TIMEOUT`], and nothing larger than the caller's limit is kept.

use anyhow::{Result, bail};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, CreateWindowAux, EventMask, Property, WindowClass};
use x11rb::rust_connection::RustConnection;

/// How long the owner may take to answer a request (as arboard waits).
const ANSWER_TIMEOUT: Duration = Duration::from_secs(4);
/// How long the owner may take between two chunks of an INCR transfer.
const CHUNK_TIMEOUT: Duration = Duration::from_secs(2);

/// The text formats, best first (arboard's order).
pub(crate) const TEXT: &[&str] = &["UTF8_STRING", "text/plain;charset=utf-8", "text/plain;charset=UTF-8", "STRING", "TEXT", "text/plain"];
pub(crate) const HTML: &[&str] = &["text/html"];

/// The first of `formats` the CLIPBOARD owner hands over, as text; `None`
/// when there is no owner or it offers none of them. Content over `limit`
/// bytes is an error (the transfer is stopped, not held in memory).
pub(crate) fn read(formats: &[&str], limit: usize) -> Result<Option<String>> {
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let atom = |name: &str| -> Result<u32> { Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom) };
    let clipboard = atom("CLIPBOARD")?;
    if conn.get_selection_owner(clipboard)?.reply()?.owner == 0 {
        return Ok(None);
    }
    let (incr, property) = (atom("INCR")?, atom("ONE_MK_CLIPBOARD")?);
    for name in formats {
        let target = atom(name)?;
        // A window of its own for each format: nothing left over from an
        // earlier answer can be taken for this one.
        let window = conn.generate_id()?;
        let aux = CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE);
        conn.create_window(0, window, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &aux)?.check()?;
        let got = read_one(&conn, window, clipboard, target, property, incr, limit);
        let _ = conn.destroy_window(window);
        let _ = conn.flush();
        if let Some(bytes) = got? {
            return Ok(Some(if *name == "STRING" { bytes.iter().map(|&b| b as char).collect() } else { String::from_utf8(bytes)? }));
        }
    }
    Ok(None)
}

fn read_one(conn: &RustConnection, window: u32, clipboard: u32, target: u32, property: u32, incr: u32, limit: usize) -> Result<Option<Vec<u8>>> {
    conn.convert_selection(window, clipboard, target, property, x11rb::CURRENT_TIME)?;
    conn.flush()?;
    let Some(event) = wait(conn, ANSWER_TIMEOUT, |e| matches!(e, Event::SelectionNotify(n) if n.requestor == window))? else {
        bail!("the clipboard's owner did not answer within {} s", ANSWER_TIMEOUT.as_secs());
    };
    let Event::SelectionNotify(notify) = event else { unreachable!() };
    if notify.property == x11rb::NONE {
        return Ok(None); // not offered in this format
    }
    // Deleting the property (delete = true) also tells an INCR owner to
    // send the first chunk.
    let reply = conn.get_property(true, window, property, AtomEnum::ANY, 0, u32::MAX / 4)?.reply()?;
    if reply.type_ != incr {
        if reply.value.len() > limit {
            bail!("{} MB is more than the clipboard limit", reply.value.len() >> 20);
        }
        // Only an answer in the format asked for (xclip answers any request
        // with its own format).
        return Ok((reply.type_ == target).then_some(reply.value));
    }
    let (mut data, mut total, mut typed) = (Vec::new(), 0usize, true);
    loop {
        let chunk = |e: &Event| matches!(e, Event::PropertyNotify(p) if p.window == window && p.atom == property && p.state == Property::NEW_VALUE);
        if wait(conn, CHUNK_TIMEOUT, chunk)?.is_none() {
            bail!("the clipboard's owner stopped sending after {} KB", total >> 10);
        }
        let reply = conn.get_property(true, window, property, AtomEnum::ANY, 0, u32::MAX / 4)?.reply()?;
        if reply.value.is_empty() {
            // A zero-length chunk ends the transfer.
            if total > limit {
                bail!("{} MB is more than the clipboard limit", total >> 20);
            }
            return Ok(typed.then_some(data));
        }
        typed &= reply.type_ == target;
        total += reply.value.len();
        // Over the limit: take the rest without keeping it. Leaving mid-way
        // would leave an owner like xclip waiting for us for good.
        if typed && total <= limit {
            data.extend_from_slice(&reply.value);
        } else {
            data = Vec::new();
        }
    }
}

/// The next event that `wanted` accepts, or `None` after `timeout`.
fn wait(conn: &RustConnection, timeout: Duration, wanted: impl Fn(&Event) -> bool) -> Result<Option<Event>> {
    let deadline = Instant::now() + timeout;
    loop {
        while let Some(event) = conn.poll_for_event()? {
            if wanted(&event) {
                return Ok(Some(event));
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        let mut fd = libc::pollfd { fd: conn.stream().as_raw_fd(), events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut fd, 1, remaining.as_millis().clamp(1, i32::MAX as u128) as i32) } < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error.into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    /// Needs a private X server (never the desktop's clipboard) and xclip:
    /// `ONE_MK_PRIVATE_X11=1 DISPLAY=:57 cargo test -p one-mk-clipboard -- --ignored x11_read`.
    #[test]
    #[ignore = "requires an explicitly private X11 display and xclip"]
    fn x11_read_takes_large_incr_text_whole_and_leaves_the_owner_answering() {
        assert_eq!(std::env::var("ONE_MK_PRIVATE_X11").as_deref(), Ok("1"));
        assert!(std::env::var_os("WAYLAND_DISPLAY").is_none());
        struct Owner(std::process::Child);
        impl Drop for Owner {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        for size in [1_000, 3_000_000] {
            let text = format!("omk-{}-end", "é".repeat(size / 2));
            let before = crate::sensitivity::owner().unwrap();
            let mut owner = Owner(std::process::Command::new("xclip").args(["-selection", "clipboard", "-in", "-quiet"])
                .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap());
            owner.0.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while crate::sensitivity::owner().unwrap() == before && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert_eq!(super::read(super::TEXT, 64 << 20).unwrap().as_deref(), Some(text.as_str()), "{size}");
            // xclip offers no HTML: nothing stale is taken for it.
            assert_eq!(super::read(super::HTML, 64 << 20).unwrap(), None, "{size}");
            // The owner still answers (arboard's abandoned transfer hung it).
            assert_eq!(super::read(super::TEXT, 64 << 20).unwrap().as_deref(), Some(text.as_str()), "{size} again");
            assert!(super::read(super::TEXT, 100).is_err(), "over the limit is refused");
            assert_eq!(super::read(super::TEXT, 64 << 20).unwrap().as_deref(), Some(text.as_str()), "{size} after a refusal");
            drop(owner);
        }
    }
}
