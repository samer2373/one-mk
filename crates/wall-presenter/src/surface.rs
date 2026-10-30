use libloading::Library;
use std::ffi::{c_char, c_void};
use std::fs::File;
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::time::{Duration, Instant};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_client::protocol::{wl_buffer, wl_callback, wl_compositor, wl_registry, wl_region, wl_shm, wl_shm_pool, wl_surface};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

struct Buffer {
    file: File,
    proxy: wl_buffer::WlBuffer,
    busy: bool,
}

struct Surface {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm: Option<xdg_wm_base::XdgWmBase>,
    surface: Option<wl_surface::WlSurface>,
    role: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    buffers: Vec<Buffer>,
    size: (u32, u32),
    serial: Option<u32>,
    configured: bool,
    frame_ready: bool,
    closed: bool,
}

impl Surface {
    fn replace(&mut self, size: (u32, u32), handle: &QueueHandle<Self>) -> Result<(), String> {
        if let Some(toplevel) = self.toplevel.take() { toplevel.destroy(); }
        if let Some(role) = self.role.take() { role.destroy(); }
        if let Some(surface) = self.surface.take() { surface.destroy(); }
        for buffer in self.buffers.drain(..) { buffer.proxy.destroy(); }
        self.serial = None;
        self.configured = false;
        self.frame_ready = true;
        self.size = size;
        let compositor = self.compositor.as_ref().ok_or("no wl_compositor")?;
        let wm = self.wm.as_ref().ok_or("no xdg_wm_base")?;
        let shm = self.shm.as_ref().ok_or("no wl_shm")?;
        let surface = compositor.create_surface(handle, ());
        let region = compositor.create_region(handle, ());
        surface.set_input_region(Some(&region));
        region.destroy();
        let role = wm.get_xdg_surface(&surface, handle, ());
        let toplevel = role.get_toplevel(handle, ());
        toplevel.set_app_id("one-mk-wall".into());
        toplevel.set_title("one-mk-wall M1a isolated probe".into());
        let bytes = u64::from(size.0) * u64::from(size.1) * 4;
        for index in 0..2 {
            let descriptor = unsafe { libc::memfd_create(c"one-mk-wall-frame".as_ptr(), libc::MFD_CLOEXEC) };
            if descriptor < 0 { return Err(std::io::Error::last_os_error().to_string()); }
            let file = unsafe { File::from_raw_fd(descriptor) };
            file.set_len(bytes).map_err(|error| error.to_string())?;
            let pool = shm.create_pool(file.as_fd(), bytes as i32, handle, ());
            let proxy = pool.create_buffer(0, size.0 as i32, size.1 as i32, size.0 as i32 * 4,
                wl_shm::Format::Xrgb8888, handle, index);
            pool.destroy();
            self.buffers.push(Buffer { file, proxy, busy: false });
        }
        surface.commit();
        self.surface = Some(surface);
        self.role = Some(role);
        self.toplevel = Some(toplevel);
        Ok(())
    }

    fn draw(&mut self, source: &[u8], handle: &QueueHandle<Self>) -> Result<(), String> {
        if !self.configured { return Ok(()); }
        let Some(buffer) = self.buffers.iter_mut().find(|buffer| !buffer.busy) else { return Ok(()); };
        let (width, height) = self.size;
        let mut pixels = vec![0; width as usize * height as usize * 4];
        if self.size == (3840, 1080) {
            pixels.copy_from_slice(source);
        } else {
            for row in 0..height as usize {
                let source_row = row * 1080 / height as usize;
                for column in 0..width as usize {
                    let source_column = column * 3840 / width as usize;
                    let origin = (source_row * 3840 + source_column) * 4;
                    let target = (row * width as usize + column) * 4;
                    pixels[target..target + 4].copy_from_slice(&source[origin..origin + 4]);
                }
            }
        }
        buffer.file.write_all_at(&pixels, 0).map_err(|error| error.to_string())?;
        let surface = self.surface.as_ref().ok_or("missing surface")?;
        if let Some(serial) = self.serial.take() {
            self.role.as_ref().ok_or("missing role")?.ack_configure(serial);
        }
        surface.attach(Some(&buffer.proxy), 0, 0);
        surface.damage_buffer(0, 0, width as i32, height as i32);
        surface.frame(handle, surface.clone());
        surface.commit();
        self.frame_ready = false;
        buffer.busy = true;
        Ok(())
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for Surface {
    fn event(state: &mut Self, registry: &wl_registry::WlRegistry, event: wl_registry::Event,
        _: &(), _: &Connection, handle: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" if version >= 4 => state.compositor = Some(registry.bind(name, 4, handle, ())),
                "wl_shm" => state.shm = Some(registry.bind(name, 1, handle, ())),
                "xdg_wm_base" => state.wm = Some(registry.bind(name, 1, handle, ())),
                _ => {},
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Surface {
    fn event(_: &mut Self, wm: &xdg_wm_base::XdgWmBase, event: xdg_wm_base::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_wm_base::Event::Ping { serial } = event { wm.pong(serial); }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for Surface {
    fn event(state: &mut Self, role: &xdg_surface::XdgSurface, event: xdg_surface::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_surface::Event::Configure { serial } = event
            && state.role.as_ref() == Some(role) {
            state.serial = Some(serial);
            state.configured = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for Surface {
    fn event(state: &mut Self, toplevel: &xdg_toplevel::XdgToplevel, event: xdg_toplevel::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_toplevel::Event::Close = event
            && state.toplevel.as_ref() == Some(toplevel) { state.closed = true; }
    }
}

impl Dispatch<wl_buffer::WlBuffer, usize> for Surface {
    fn event(state: &mut Self, proxy: &wl_buffer::WlBuffer, event: wl_buffer::Event,
        index: &usize, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_buffer::Event::Release = event
            && let Some(buffer) = state.buffers.get_mut(*index).filter(|buffer| buffer.proxy == *proxy) {
                buffer.busy = false;
        }
    }
}

impl Dispatch<wl_callback::WlCallback, wl_surface::WlSurface> for Surface {
    fn event(state: &mut Self, _: &wl_callback::WlCallback, event: wl_callback::Event,
        surface: &wl_surface::WlSurface, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_callback::Event::Done { .. } = event
            && state.surface.as_ref() == Some(surface) {
            state.frame_ready = true;
        }
    }
}

delegate_noop!(Surface: ignore wl_compositor::WlCompositor);
delegate_noop!(Surface: ignore wl_shm::WlShm);
delegate_noop!(Surface: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Surface: ignore wl_surface::WlSurface);
delegate_noop!(Surface: ignore wl_region::WlRegion);

fn extent(monitors: &[(i32, i32, i32, i32)]) -> Result<(u32, u32), String> {
    if monitors.is_empty() || monitors.len() > 16 || monitors.iter().any(|monitor| monitor.2 <= 0 || monitor.3 <= 0) {
        return Err("invalid isolated probe monitor count or extent".into());
    }
    let left = monitors.iter().map(|monitor| i64::from(monitor.0)).min().ok_or("no monitors")?;
    let top = monitors.iter().map(|monitor| i64::from(monitor.1)).min().ok_or("no monitors")?;
    let right = monitors.iter().map(|monitor| i64::from(monitor.0) + i64::from(monitor.2)).max().ok_or("no monitors")?;
    let bottom = monitors.iter().map(|monitor| i64::from(monitor.1) + i64::from(monitor.3)).max().ok_or("no monitors")?;
    let (width, height) = (right - left, bottom - top);
    if width > 16384 || height > 16384 || width * height > 64_000_000 {
        return Err("isolated probe canvas exceeds pixel budget".into());
    }
    Ok((width as u32, height as u32))
}

pub unsafe fn present(_root: &Path, gst: &Library, pipeline: *mut c_void) -> Result<(), String> {
    unsafe {
        let app = Library::new("libgstapp-1.0.so.0").map_err(|error| error.to_string())?;
        let get: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void = *gst.get(b"gst_bin_get_by_name\0").map_err(|error| error.to_string())?;
        let pull: unsafe extern "C" fn(*mut c_void, u64) -> *mut c_void = *app.get(b"gst_app_sink_try_pull_sample\0").map_err(|error| error.to_string())?;
        let buffer: unsafe extern "C" fn(*mut c_void) -> *mut c_void = *gst.get(b"gst_sample_get_buffer\0").map_err(|error| error.to_string())?;
        let extract: unsafe extern "C" fn(*mut c_void, usize, *mut c_void, usize) -> usize = *gst.get(b"gst_buffer_extract\0").map_err(|error| error.to_string())?;
        let unref: unsafe extern "C" fn(*mut c_void) = *gst.get(b"gst_mini_object_unref\0").map_err(|error| error.to_string())?;
        let object_unref: unsafe extern "C" fn(*mut c_void) = *gst.get(b"gst_object_unref\0").map_err(|error| error.to_string())?;
        let eos: unsafe extern "C" fn(*mut c_void) -> i32 = *app.get(b"gst_app_sink_is_eos\0").map_err(|error| error.to_string())?;
        let seek: unsafe extern "C" fn(*mut c_void, i32, i32, i64) -> i32 = *gst.get(b"gst_element_seek_simple\0").map_err(|error| error.to_string())?;
        let set_state: unsafe extern "C" fn(*mut c_void, i32) -> i32 = *gst.get(b"gst_element_set_state\0").map_err(|error| error.to_string())?;
        let sink = get(pipeline, c"frames".as_ptr());
        if sink.is_null() { return Err("missing appsink".into()); }
        let result = (|| {
            let mut bridge = crate::bridge::Bridge::new()?;
            let monitors = bridge.monitors.clone();
            let connection = Connection::connect_to_env().map_err(|error| error.to_string())?;
            let mut queue = connection.new_event_queue();
            let handle = queue.handle();
            connection.display().get_registry(&handle, ());
            let mut state = Surface { compositor: None, shm: None, wm: None, surface: None,
                role: None, toplevel: None, buffers: Vec::new(), size: (3840, 1080),
                serial: None, configured: false, frame_ready: true, closed: false };
            queue.roundtrip(&mut state).map_err(|error| error.to_string())?;
            state.replace(extent(&monitors)?, &handle)?;
            let start = Instant::now();
            let mut paused = false;
            let mut pixels = vec![0; 3840 * 1080 * 4];
            while !state.closed && start.elapsed() < Duration::from_secs(600) {
                queue.roundtrip(&mut state).map_err(|error| error.to_string())?;
                for event in bridge.poll() {
                    if let crate::bridge::Event::Layout(monitors) = event {
                        let size = extent(&monitors)?;
                        if state.size != size { state.replace(size, &handle)?; }
                    } else if let crate::bridge::Event::Pause(pause) = event
                        && paused != pause {
                            if set_state(pipeline, if pause { 3 } else { 4 }) == 0 { return Err("pause transition failed".into()); }
                            paused = pause;
                    }
                }
                if !paused && state.configured && state.frame_ready && state.buffers.iter().any(|buffer| !buffer.busy) {
                    let sample = pull(sink, 0);
                    if !sample.is_null() {
                        let frame = buffer(sample);
                        let copied = if frame.is_null() { 0 } else { extract(frame, 0, pixels.as_mut_ptr().cast(), pixels.len()) };
                        unref(sample);
                        if copied != pixels.len() { return Err("unexpected fixed-fixture frame size".into()); }
                        state.draw(&pixels, &handle)?;
                    }
                    if eos(sink) != 0 && seek(pipeline, 3, 5, 0) == 0 { return Err("fixture loop seek failed".into()); }
                }
                if paused {
                    bridge.wait(connection.as_fd().as_raw_fd())?;
                } else {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            Ok(())
        })();
        object_unref(sink);
        result
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn extent_is_bounded() {
        for invalid in [vec![], vec![(0, 0, 0, 1080)], vec![(0, 0, 16385, 1)],
            vec![(0, 0, 16384, 16384)], vec![(0, 0, -1, 2)], vec![(0, 0, 1, 1); 17],
            vec![(i32::MIN, 0, 1, 1), (i32::MAX, 0, 1, 1)]] {
            assert!(super::extent(&invalid).is_err());
        }
        assert_eq!(super::extent(&[(-1920, 0, 1920, 1080), (0, 0, 1920, 1080)]).unwrap(), (3840, 1080));
    }
}
