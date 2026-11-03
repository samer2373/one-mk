use super::*;
use std::collections::BTreeMap;
use std::fs::File;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop, protocol::{wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface}};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};
use wayland_protocols::xdg::shell::client::{xdg_wm_base, xdg_surface, xdg_toplevel};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};
use one_mk_wallpaper::{Monitor, Playback, Rect};

#[derive(Default)]
struct Desktop {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    windows: Option<xdg_wm_base::XdgWmBase>,
    secret: Option<String>,
    manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
    outputs: BTreeMap<u32, Output>,
    changed: bool,
}

struct Output {
    proxy: wl_output::WlOutput,
    xdg: Option<zxdg_output_v1::ZxdgOutputV1>,
    name: String,
    xdg_name: String,
    rect: Rect,
    logical_size: Option<(i32, i32)>,
    logical_position: Option<(i32, i32)>,
    mode: (i32, i32),
    rotated: bool,
    scale: i32,
    done: bool,
    wallpaper: Option<Wallpaper>,
    closed: bool,
}

struct Wallpaper {
    surface: wl_surface::WlSurface,
    role: Role,
    rect: Rect,
    title: String,
    presented: bool,
    configured: bool,
    size: (u32, u32),
    scale: i32,
    buffers: Vec<Buffer>,
    pixels: Vec<u8>,
    mapping: PixelMapping,
    generation: u64,
}

enum Role { Layer(zwlr_layer_surface_v1::ZwlrLayerSurfaceV1), Window(xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel) }

fn window_title(secret: &str, id: u32, rect: Rect) -> String {
    format!("oneMK:{secret}:{id}:{}:{}:{}:{}", rect.x as i32, rect.y as i32, rect.width as u32, rect.height as u32)
}

struct Buffer { proxy: wl_buffer::WlBuffer, file: File, free: Arc<AtomicBool>, size: (u32, u32) }
#[derive(Default)]
struct PixelMapping { columns: Vec<Option<usize>>, rows: Vec<Option<usize>> }
impl PixelMapping { fn is_empty(&self) -> bool { self.columns.is_empty() || self.rows.is_empty() } }
impl Drop for Buffer { fn drop(&mut self) { self.proxy.destroy(); } }
impl Drop for Wallpaper { fn drop(&mut self) {
    match &self.role { Role::Layer(layer) => layer.destroy(), Role::Window(surface, window) => { window.destroy(); surface.destroy(); } }
    self.surface.destroy();
} }
impl Drop for Output {
    fn drop(&mut self) {
        self.wallpaper.take();
        if let Some(xdg) = self.xdg.take() { xdg.destroy(); }
        if self.proxy.version() >= 3 { self.proxy.release(); }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for Desktop {
    fn event(state: &mut Self, registry: &wl_registry::WlRegistry, event: wl_registry::Event, _: &(), _: &Connection, handle: &QueueHandle<Self>) {
        match event {
            wl_registry::Event::Global { name, interface, version } => match interface.as_str() {
                "wl_compositor" => state.compositor = Some(registry.bind(name, version.min(4), handle, ())),
                "wl_shm" => state.shm = Some(registry.bind(name, 1, handle, ())),
                "zwlr_layer_shell_v1" => state.shell = Some(registry.bind(name, version.min(4), handle, ())),
                "xdg_wm_base" => state.windows = Some(registry.bind(name, version.min(3), handle, ())),
                "zxdg_output_manager_v1" => {
                    let manager: zxdg_output_manager_v1::ZxdgOutputManagerV1 = registry.bind(name, version.min(3), handle, ());
                    for (id, output) in &mut state.outputs { output.xdg = Some(manager.get_xdg_output(&output.proxy, handle, *id)); }
                    state.manager = Some(manager);
                }
                "wl_output" => {
                    let proxy = registry.bind(name, version.min(4), handle, name);
                    let xdg = state.manager.as_ref().map(|manager| manager.get_xdg_output(&proxy, handle, name));
                    state.outputs.insert(name, Output { proxy, xdg, name: String::new(), xdg_name: String::new(), rect: Rect::default(),
                        logical_size: None, logical_position: None, mode: (0, 0), rotated: false, scale: 1, done: false, wallpaper: None, closed: false });
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => { state.outputs.remove(&name); }
            _ => {}
        }
        state.changed = true;
    }
}

impl Output {
    fn update_rect(&mut self) {
        let (width, height) = if self.rotated { (self.mode.1, self.mode.0) } else { self.mode };
        let (width, height) = self.logical_size.unwrap_or((width / self.scale, height / self.scale));
        self.rect.width = f64::from(width); self.rect.height = f64::from(height);
        if let Some((horizontal, vertical)) = self.logical_position { self.rect.x = f64::from(horizontal); self.rect.y = f64::from(vertical); }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for Desktop {
    fn event(state: &mut Self, _: &wl_output::WlOutput, event: wl_output::Event, id: &u32, _: &Connection, _: &QueueHandle<Self>) {
        let Some(output) = state.outputs.get_mut(id) else { return };
        match event {
            wl_output::Event::Name { name } => output.name = name,
            wl_output::Event::Geometry { x, y, transform, .. } => {
                output.rect.x = f64::from(x); output.rect.y = f64::from(y);
                output.rotated = matches!(transform, WEnum::Value(wl_output::Transform::_90 | wl_output::Transform::_270 | wl_output::Transform::Flipped90 | wl_output::Transform::Flipped270));
            }
            wl_output::Event::Mode { flags: WEnum::Value(flags), width, height, .. } if flags.contains(wl_output::Mode::Current) => output.mode = (width, height),
            wl_output::Event::Scale { factor } => output.scale = factor.max(1),
            wl_output::Event::Done => { output.done = true; output.update_rect(); state.changed = true; }
            _ => {}
        }
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, u32> for Desktop {
    fn event(state: &mut Self, _: &zxdg_output_v1::ZxdgOutputV1, event: zxdg_output_v1::Event, id: &u32, _: &Connection, _: &QueueHandle<Self>) {
        let Some(output) = state.outputs.get_mut(id) else { return };
        match event {
            zxdg_output_v1::Event::Name { name } => output.xdg_name = name,
            zxdg_output_v1::Event::LogicalPosition { x, y } => output.logical_position = Some((x, y)),
            zxdg_output_v1::Event::LogicalSize { width, height } => output.logical_size = Some((width, height)),
            _ => {}
        }
        output.update_rect(); state.changed = true;
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, u32> for Desktop {
    fn event(state: &mut Self, layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, event: zwlr_layer_surface_v1::Event, id: &u32, _: &Connection, _: &QueueHandle<Self>) {
        let Some(output) = state.outputs.get_mut(id) else { return };
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                layer.ack_configure(serial);
                if let Some(wallpaper) = &mut output.wallpaper {
                    wallpaper.size = (if width == 0 { output.rect.width as u32 } else { width }, if height == 0 { output.rect.height as u32 } else { height }); wallpaper.generation = 0;
                    wallpaper.configured = true;
                }
            }
            zwlr_layer_surface_v1::Event::Closed => { output.wallpaper.take(); output.closed = true; }
            _ => {}
        }
        state.changed = true;
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Desktop {
    fn event(_: &mut Self, shell: &xdg_wm_base::XdgWmBase, event: xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_wm_base::Event::Ping { serial } = event { shell.pong(serial); }
    }
}
impl Dispatch<xdg_surface::XdgSurface, u32> for Desktop {
    fn event(state: &mut Self, surface: &xdg_surface::XdgSurface, event: xdg_surface::Event, id: &u32, _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
            if let Some(wallpaper) = state.outputs.get_mut(id).and_then(|output| output.wallpaper.as_mut()) {
                wallpaper.configured = true; wallpaper.generation = 0;
            }
            state.changed = true;
        }
    }
}
impl Dispatch<xdg_toplevel::XdgToplevel, u32> for Desktop {
    fn event(state: &mut Self, _: &xdg_toplevel::XdgToplevel, event: xdg_toplevel::Event, id: &u32, _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_toplevel::Event::Close = event && let Some(output) = state.outputs.get_mut(id) {
            output.wallpaper.take(); output.closed = true; state.changed = true;
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, Arc<AtomicBool>> for Desktop {
    fn event(_: &mut Self, _: &wl_buffer::WlBuffer, event: wl_buffer::Event, free: &Arc<AtomicBool>, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_buffer::Event::Release = event { free.store(true, Ordering::Release); }
    }
}
delegate_noop!(Desktop: ignore wl_compositor::WlCompositor);
delegate_noop!(Desktop: ignore wl_shm::WlShm);
delegate_noop!(Desktop: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Desktop: ignore wl_surface::WlSurface);
delegate_noop!(Desktop: ignore zxdg_output_manager_v1::ZxdgOutputManagerV1);
delegate_noop!(Desktop: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

fn pixel_size(size: (u32, u32), scale: i32) -> Result<(u32, u32)> {
    let width = size.0.checked_mul(scale as u32).context("Wallpaper output is too wide")?;
    let height = size.1.checked_mul(scale as u32).context("Wallpaper output is too tall")?;
    ensure!(width > 0 && height > 0 && width <= 32768 && height <= 32768 && u64::from(width) * u64::from(height) <= 64 * 1024 * 1024,
        "This Wayland output is too large for a wallpaper surface");
    Ok((width, height))
}

impl Buffer {
    fn new(shm: &wl_shm::WlShm, size: (u32, u32), handle: &QueueHandle<Desktop>) -> Result<Self> {
        let bytes = u64::from(size.0) * u64::from(size.1) * 4;
        let descriptor = unsafe { libc::memfd_create(c"oneMK wallpaper".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
        ensure!(descriptor >= 0, "Cannot allocate Wayland wallpaper memory: {}", std::io::Error::last_os_error());
        let file = unsafe { File::from_raw_fd(descriptor) }; file.set_len(bytes)?;
        ensure!(unsafe { libc::fcntl(descriptor, libc::F_ADD_SEALS, libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL) } >= 0,
            "Cannot seal Wayland wallpaper memory");
        let pool = shm.create_pool(file.as_fd(), bytes as i32, handle, ());
        let free = Arc::new(AtomicBool::new(true));
        let proxy = pool.create_buffer(0, size.0 as i32, size.1 as i32, (size.0 * 4) as i32, wl_shm::Format::Xrgb8888, handle, free.clone());
        pool.destroy();
        Ok(Self { proxy, file, free, size })
    }
}

fn mapping(config: &PlayerConfig, monitor: &Monitor, size: (u32, u32), source: (u32, u32)) -> Result<PixelMapping> {
    let slice = config.plan.canvas.slice(monitor, &config.plan.media, config.plan.fit).context("Empty wallpaper canvas")?;
    let horizontal: Vec<_> = (0..size.0).map(|column| {
        let value = slice.x + (f64::from(column) + 0.5) / f64::from(size.0) * slice.width;
        (0.0..1.0).contains(&value).then_some((value * f64::from(source.0)).floor() as usize * 4)
    }).collect();
    let rows = (0..size.1).map(|row| {
        let value = slice.y + (f64::from(row) + 0.5) / f64::from(size.1) * slice.height;
        (0.0..1.0).contains(&value).then_some((value * f64::from(source.1)).floor() as usize * source.0 as usize * 4)
    }).collect();
    Ok(PixelMapping { columns: horizontal, rows })
}

impl Desktop {
    fn reconcile(&mut self, config: &PlayerConfig, handle: &QueueHandle<Self>, source: (u32, u32)) -> Result<()> {
        let compositor = self.compositor.as_ref().context("The Wayland compositor has no surface interface")?;
        for (id, output) in &mut self.outputs {
            if !output.done || output.closed { continue; }
            let Some(monitor) = one_mk_wallpaper::wayland::match_monitor(&config.plan.canvas.monitors, &config.computer, &[&output.name, &output.xdg_name], output.rect) else {
                output.wallpaper.take(); continue;
            };
            if self.secret.is_some() && output.wallpaper.as_ref().is_some_and(|wallpaper| wallpaper.rect != output.rect) { output.wallpaper.take(); }
            if output.wallpaper.is_none() {
                let surface = compositor.create_surface(handle, ());
                let title = self.secret.as_ref().map(|secret| window_title(secret, *id, output.rect)).unwrap_or_default();
                let role = if self.secret.is_some() {
                    let shell = self.windows.as_ref().context("GNOME does not provide xdg-shell windows")?;
                    let xdg = shell.get_xdg_surface(&surface, handle, *id);
                    let window = xdg.get_toplevel(handle, *id);
                    window.set_app_id("org.one-mk.Wallpaper".into()); window.set_title(title.clone());
                    window.set_min_size(output.rect.width as i32, output.rect.height as i32);
                    window.set_max_size(output.rect.width as i32, output.rect.height as i32);
                    Role::Window(xdg, window)
                } else {
                    let shell = self.shell.as_ref().context(one_mk_wallpaper::wayland::UNSUPPORTED)?;
                    let layer = shell.get_layer_surface(&surface, Some(&output.proxy), zwlr_layer_shell_v1::Layer::Background, "oneMK wallpaper".into(), handle, *id);
                    layer.set_anchor(zwlr_layer_surface_v1::Anchor::Top | zwlr_layer_surface_v1::Anchor::Bottom | zwlr_layer_surface_v1::Anchor::Left | zwlr_layer_surface_v1::Anchor::Right);
                    layer.set_exclusive_zone(-1); layer.set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None); layer.set_size(0, 0);
                    Role::Layer(layer)
                };
                let region = compositor.create_region(handle, ()); surface.set_input_region(Some(&region)); region.destroy();
                surface.commit();
                output.wallpaper = Some(Wallpaper { surface, role, rect: output.rect, title, presented: self.secret.is_none(), configured: false,
                    size: (output.rect.width as u32, output.rect.height as u32), scale: output.scale, buffers: Vec::new(), pixels: Vec::new(), mapping: PixelMapping::default(), generation: 0 });
            }
            let wallpaper = output.wallpaper.as_mut().unwrap();
            if !wallpaper.configured || wallpaper.size.0 == 0 || wallpaper.size.1 == 0 { continue; }
            // One output too large to paint must not take the others down.
            let Ok(size) = pixel_size(wallpaper.size, output.scale) else {
                eprintln!("oneMK wallpaper: skipping output {id}, {}x{} at scale {} is too large", wallpaper.size.0, wallpaper.size.1, output.scale);
                output.wallpaper.take(); output.closed = true; continue;
            };
            if wallpaper.scale != output.scale || wallpaper.pixels.len() != size.0 as usize * size.1 as usize * 4 || wallpaper.generation == 0 {
                wallpaper.scale = output.scale; wallpaper.surface.set_buffer_scale(output.scale);
                wallpaper.mapping = mapping(config, monitor, size, source)?;
                wallpaper.pixels.resize(size.0 as usize * size.1 as usize * 4, 0);
                wallpaper.buffers.retain(|buffer| buffer.size == size);
                wallpaper.generation = 0;
            }
        }
        Ok(())
    }

    fn paint(&mut self, frame: &[u8], generation: u64, handle: &QueueHandle<Self>) -> Result<()> {
        let shm = self.shm.as_ref().context("The Wayland compositor has no shared-memory wallpaper interface")?;
        for output in self.outputs.values_mut() {
            let Some(wallpaper) = &mut output.wallpaper else { continue };
            if wallpaper.mapping.is_empty() || wallpaper.generation == generation { continue; }
            let size = pixel_size(wallpaper.size, wallpaper.scale)?;
            if !wallpaper.buffers.iter().any(|buffer| buffer.free.load(Ordering::Acquire)) && wallpaper.buffers.len() < 3 {
                wallpaper.buffers.push(Buffer::new(shm, size, handle)?);
            }
            let Some(buffer) = wallpaper.buffers.iter().find(|buffer| buffer.free.load(Ordering::Acquire)) else { continue };
            for (row, source_row) in wallpaper.pixels.chunks_exact_mut(size.0 as usize * 4).zip(&wallpaper.mapping.rows) {
                for (pixel, column) in row.as_chunks_mut::<4>().0.iter_mut().zip(&wallpaper.mapping.columns) {
                    if let Some(source) = column.zip(*source_row).map(|(column, row)| row + column) { pixel.copy_from_slice(&frame[source..source + 4]); }
                    else { pixel.copy_from_slice(&[0, 0, 0, 255]); }
                }
            }
            buffer.file.write_all_at(&wallpaper.pixels, 0)?;
            buffer.free.store(false, Ordering::Release);
            wallpaper.surface.attach(Some(&buffer.proxy), 0, 0);
            if wallpaper.surface.version() >= 4 { wallpaper.surface.damage_buffer(0, 0, size.0 as i32, size.1 as i32); }
            else { wallpaper.surface.damage(0, 0, wallpaper.size.0 as i32, wallpaper.size.1 as i32); }
            wallpaper.surface.commit(); wallpaper.generation = generation;
        }
        Ok(())
    }
}
delegate_noop!(Desktop: ignore wayland_client::protocol::wl_region::WlRegion);

struct Samples {
    _app: Library,
    pull: unsafe extern "C" fn(Object, u64) -> Object,
    preroll: unsafe extern "C" fn(Object, u64) -> Object,
    buffer: unsafe extern "C" fn(Object) -> Object,
    caps: unsafe extern "C" fn(Object) -> Object,
    extract: unsafe extern "C" fn(Object, usize, *mut c_void, usize) -> usize,
}

impl Samples {
    fn load(gst: &Gst) -> Result<Self> {
        unsafe {
            let app = Library::new("libgstapp-1.0.so.0").context("Install the GStreamer appsink library for Wayland video wallpapers")?;
            Ok(Self {
                pull: api!(app, "gst_app_sink_try_pull_sample", unsafe extern "C" fn(Object, u64) -> Object),
                preroll: api!(app, "gst_app_sink_try_pull_preroll", unsafe extern "C" fn(Object, u64) -> Object),
                buffer: api!(gst._core, "gst_sample_get_buffer", unsafe extern "C" fn(Object) -> Object),
                caps: api!(gst._core, "gst_sample_get_caps", unsafe extern "C" fn(Object) -> Object),
                extract: api!(gst._core, "gst_buffer_extract", unsafe extern "C" fn(Object, usize, *mut c_void, usize) -> usize),
                _app: app,
            })
        }
    }

    fn frame(&self, gst: &Gst, sink: Object, preroll: bool, size: (u32, u32)) -> Result<Option<Vec<u8>>> {
        let sample = unsafe { (if preroll { self.preroll } else { self.pull })(sink, 0) };
        if sample.is_null() { return Ok(None); }
        let result = (|| {
            let caps = unsafe { (self.caps)(sample) };
            ensure!(!caps.is_null(), "GStreamer returned a frame without dimensions");
            let structure = unsafe { (gst.structure)(caps, 0) };
            let (mut width, mut height) = (0, 0);
            unsafe { (gst.integer)(structure, c"width".as_ptr(), &mut width); (gst.integer)(structure, c"height".as_ptr(), &mut height); }
            ensure!((width as u32, height as u32) == size, "The video dimensions changed during playback");
            let buffer = unsafe { (self.buffer)(sample) };
            ensure!(!buffer.is_null(), "GStreamer returned an empty wallpaper frame");
            let mut pixels = vec![0; size.0 as usize * size.1 as usize * 4];
            ensure!(unsafe { (self.extract)(buffer, 0, pixels.as_mut_ptr().cast(), pixels.len()) } == pixels.len(), "GStreamer returned an incomplete wallpaper frame");
            Ok(Some(pixels))
        })();
        unsafe { (gst.mini_unref)(sample); }
        result
    }
}

unsafe extern "C" fn sample_ready(_: Object, _: Object) -> c_int { wake(); 0 }

pub(super) fn run(mut config: PlayerConfig, receiver: Receiver<Input>) -> Result<()> {
    let connection = Connection::connect_to_env().context("Cannot connect to the Wayland compositor")?;
    let mut queue = connection.new_event_queue(); let handle = queue.handle();
    connection.display().get_registry(&handle, ());
    let mut desktop = Desktop::default();
    queue.roundtrip(&mut desktop)?; queue.roundtrip(&mut desktop)?;
    let bridge = if one_mk_wallpaper::gnome_wayland() { Some(super::gnome::Bridge::start()?) } else { None };
    desktop.secret = bridge.as_ref().map(|bridge| bridge.secret.clone());
    one_mk_wallpaper::wayland::capabilities(desktop.shell.is_some() || (bridge.is_some() && desktop.windows.is_some()), desktop.outputs.len()).map_err(anyhow::Error::msg)?;
    let source = (config.plan.media.width, config.plan.media.height);
    pixel_size(source, 1)?;
    let mut generation = 1u64;
    let picture = if config.plan.picture {
        let image = crate::picture::player_source(&config)?;
        let mut pixels = image.into_raw();
        for pixel in pixels.as_chunks_mut::<4>().0 {
            for channel in 0..3 { pixel[channel] = ((u16::from(pixel[channel]) * u16::from(pixel[3]) + 127) / 255) as u8; }
            pixel.swap(0, 2); pixel[3] = 255;
        }
        Some(pixels)
    } else { None };
    let gst = if picture.is_none() { Some(Gst::load()?) } else { None };
    let samples = gst.as_ref().map(Samples::load).transpose()?;
    let pipeline = gst.as_ref().map(|gst| gst.pipeline(&format!(
        "uridecodebin uri={} caps=video/x-raw name=source source. ! queue max-size-buffers=2 max-size-bytes=0 max-size-time=0 ! videoconvert ! video/x-raw,format=BGRx,width={},height={},pixel-aspect-ratio=1/1 ! appsink name=frames sync=true emit-signals=true max-buffers=1 drop=true enable-last-sample=false",
        quoted(&uri(&config.path)?), source.0, source.1))).transpose()?;
    let mut sink = ptr::null_mut(); let mut bus = ptr::null_mut();
    let mut bus_descriptor = PollDescriptor { fd: -1, events: 0, revents: 0 };
    let clock = gst.as_ref().map(|gst| Clock { gst, raw: unsafe { (gst.clock)() } });
    if let (Some(gst), Some(pipeline), Some(clock)) = (&gst, &pipeline, &clock) {
        sink = gst.object(pipeline.raw, "frames")?;
        unsafe {
            let connect = api!(gst._object, "g_signal_connect_data", unsafe extern "C" fn(Object, *const c_char, unsafe extern "C" fn(Object, Object) -> c_int, Object, Object, c_uint) -> c_ulong);
            for signal in [c"new-sample", c"new-preroll"] { connect(sink, signal.as_ptr(), sample_ready, ptr::null_mut(), ptr::null_mut(), 0); }
            (gst.use_clock)(pipeline.raw, clock.raw);
            bus = (gst.bus)(pipeline.raw); (gst.poll)(bus, &mut bus_descriptor);
        }
        pipeline.preroll()?;
        pipeline.segment(position(now_ms(), config.clock_offset_ms, config.plan.start_ms, config.plan.media.duration_ms), config.plan.media.duration_ms, true);
        pipeline.preroll()?;
    }
    let result = (|| -> Result<()> {
        let mut frame = picture;
        if let (Some(gst), Some(samples)) = (&gst, &samples) { frame = samples.frame(gst, sink, true, source)?; }
        let mut holds = Holds::default(); holds.set(Hold::User, config.plan.paused);
        let power = power_events();
        let mut power_changed = true;
        let mut playing = false;
        let mut last_sync = Instant::now(); let mut rate = 1.0;
        let mut last_report = String::new();
        let mut pending_since = None;
        loop {
            ensure!(bridge.as_ref().is_none_or(|bridge| bridge.alive()), "GNOME extension was disabled or the Shell session ended; wallpaper stopped");
            let mut resync = false;
            while let Ok(input) = receiver.try_recv() {
                match input {
                    Input::End => return Ok(()),
                    Input::Offset(offset) => { config.clock_offset_ms = offset; resync = true; }
                    Input::Pause(paused) => { config.plan.paused = paused; holds.set(Hold::User, paused); }
                }
            }
            queue.dispatch_pending(&mut desktop)?;
            if desktop.changed { desktop.reconcile(&config, &handle, source)?; desktop.changed = false; }
            let active = desktop.outputs.values().any(|output| output.wallpaper.as_ref().is_some_and(|wallpaper| !wallpaper.mapping.is_empty()));
            if power_changed { holds.set(Hold::Battery, config.plan.pause_on_battery && on_battery()); power_changed = false; }
            let should_play = !config.plan.picture && holds.playing() && active;
            if let (Some(gst), Some(pipeline)) = (&gst, &pipeline) {
                if playing != should_play || (resync && should_play) {
                    if should_play { pipeline.segment(position(now_ms(), config.clock_offset_ms, config.plan.start_ms, config.plan.media.duration_ms), config.plan.media.duration_ms, true); }
                    ensure!(unsafe { (gst.state)(pipeline.raw, if should_play { 4 } else { 3 }) } != 0, "GStreamer could not change wallpaper playback state");
                    playing = should_play;
                }
                if playing && last_sync.elapsed() >= Duration::from_secs(2) {
                    let mut actual = 0;
                    let expected = position(now_ms(), config.clock_offset_ms, config.plan.start_ms, config.plan.media.duration_ms);
                    if unsafe { (gst.position)(pipeline.raw, 3, &mut actual) } != 0 {
                        match correction(actual.max(0) as u64 / 1_000_000, expected, config.plan.media.duration_ms) {
                            Correction::Seek(position) => { if let Some(clock) = &clock { clock.rate(1.0); } rate = 1.0; pipeline.segment(position, config.plan.media.duration_ms, true); }
                            adjustment => {
                                let target = if let Correction::Rate(rate) = adjustment { rate } else { 1.0 };
                                if target != rate { if let Some(clock) = &clock { clock.rate(target); } rate = target; }
                            }
                        }
                    }
                    last_sync = Instant::now();
                }
                loop {
                    let message = unsafe { (gst.pop)(bus, 0, 1 | 2 | (1 << 17)) };
                    if message.is_null() { break; }
                    let error = unsafe { (gst.has_name)(message, c"GstMessageError".as_ptr()) != 0 };
                    let segment = unsafe { (gst.has_name)(message, c"GstMessageSegmentDone".as_ptr()) != 0 };
                    unsafe { (gst.mini_unref)(message); }
                    ensure!(!error, "GStreamer stopped playing this video. Try a video your system can decode.");
                    pipeline.segment(0, config.plan.media.duration_ms, !segment);
                }
            }
            if let (Some(gst), Some(samples)) = (&gst, &samples) {
                if let Some(pixels) = samples.frame(gst, sink, false, source)? {
                    frame = Some(pixels); generation += 1;
                } else if frame.is_none() && let Some(pixels) = samples.frame(gst, sink, true, source)? {
                    frame = Some(pixels); generation += 1;
                }
            }
            if let Some(frame) = &frame { desktop.paint(frame, generation, &handle)?; }
            if let Some(bridge) = &bridge && desktop.outputs.values().any(|output| output.wallpaper.as_ref().is_some_and(|wallpaper| wallpaper.generation > 0 && !wallpaper.presented)) {
                let titles = bridge.presented()?;
                for output in desktop.outputs.values_mut() {
                    if let Some(wallpaper) = &mut output.wallpaper { wallpaper.presented = titles.contains(&wallpaper.title); }
                }
            }
            let names = desktop.outputs.values().filter(|output| output.wallpaper.as_ref().is_some_and(|wallpaper| wallpaper.generation > 0 && wallpaper.presented))
                .map(|output| if output.name.is_empty() { output.xdg_name.as_str() } else { output.name.as_str() }).collect::<Vec<_>>().join(", ");
            let pending = desktop.outputs.values().any(|output| output.wallpaper.as_ref().is_some_and(|wallpaper| wallpaper.generation == 0 || !wallpaper.presented));
            if pending { ensure!(pending_since.get_or_insert_with(Instant::now).elapsed() < Duration::from_secs(5), "The compositor did not present all selected wallpaper monitors within five seconds"); }
            else { pending_since = None; }
            let message = if pending { "Preparing Wayland wallpaper surfaces…".into() }
                else if !active { "No selected monitors match connected Wayland outputs; waiting for an output".into() }
                else if config.plan.picture { format!("Picture shown by oneMK on {names}; your own wallpaper returns when turned off") }
                else if holds.playing() { format!("Playing video on Wayland outputs: {names}") }
                else { format!("{} on Wayland outputs: {names}", holds.message()) };
            if message != last_report {
                one_mk_wallpaper::event(if pending { Playback::Preparing } else if !active { Playback::Error(message.clone()) } else if config.plan.picture { Playback::Picture }
                    else if !holds.playing() { Playback::Paused(message.clone()) } else { Playback::Ready });
                if let Some(directory) = config.path.parent() { crate::picture::atomic_write(&directory.join("playback.txt"), message.as_bytes())?; }
                eprintln!("{message}"); last_report = message;
            }
            let Some(guard) = queue.prepare_read() else { continue };
            let writable = match connection.flush() {
                Ok(()) => false,
                Err(wayland_client::backend::WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => true,
                Err(error) => return Err(error.into()),
            };
            let mut descriptors = [
                libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN | if writable { libc::POLLOUT } else { 0 }, revents: 0 },
                libc::pollfd { fd: input_socket().0.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: bus_descriptor.fd, events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: power.as_ref().map_or(-1, AsRawFd::as_raw_fd), events: libc::POLLIN, revents: 0 },
            ];
            let polled = unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as _, if pending { 100 } else if playing { 2000 } else { -1 }) };
            if polled < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted { return Err(std::io::Error::last_os_error().into()); }
            if descriptors[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
                match guard.read() {
                    Ok(_) => {}
                    Err(wayland_client::backend::WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error.into()),
                }
            } else { drop(guard); }
            let mut bytes = [0u8; 4096];
            while (&input_socket().0).read(&mut bytes).is_ok_and(|count| count > 0) {}
            if let Some(power) = &power { unsafe {
                loop {
                    let count = libc::recv(power.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len(), libc::MSG_DONTWAIT);
                    if count <= 0 { break; }
                    if bytes[..count as usize].split(|byte| *byte == 0).any(|field| field == b"SUBSYSTEM=power_supply") { power_changed = true; }
                }
            } }
        }
    })();
    if let Some(gst) = &gst { unsafe { (gst.unref)(sink); (gst.unref)(bus); } }
    drop(pipeline); drop(desktop); let _ = connection.flush();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gnome_window_identity_carries_launch_and_logical_geometry_not_monitor_names() {
        let secret = "0123456789abcdef0123456789abcdef";
        let title = window_title(secret, 7, Rect { x: -1920.0, y: -360.0, width: 1920.0, height: 1080.0 });
        assert_eq!(title, "oneMK:0123456789abcdef0123456789abcdef:7:-1920:-360:1920:1080");
        assert_eq!(&title[39..], "7:-1920:-360:1920:1080");
    }
    #[test]
    fn dimensions_respect_scale_and_bound_memory() {
        assert_eq!(pixel_size((800, 600), 2).unwrap(), (1600, 1200));
        assert!(pixel_size((32768, 32768), 1).is_err());
        assert!(pixel_size((u32::MAX, 1), 2).is_err());
    }
    fn config(fit: one_mk_wallpaper::Fit) -> PlayerConfig {
        let monitors = (0..2).map(|index| Monitor { computer: "pc".into(), name: format!("DP-{index}"),
            rect: Rect { x: f64::from(index * 2 - 2), y: -1.0, width: 2.0, height: 4.0 }, local: None, scale: 1.0, online: true }).collect();
        PlayerConfig { computer: "pc".into(), path: "/tmp/test.video".into(), clock_offset_ms: 0, plan: one_mk_wallpaper::Plan {
            hash: "a".repeat(64), title: "test".into(), media: Media { width: 4, height: 2, duration_ms: 1000, frame_rate: 30 },
            canvas: one_mk_wallpaper::Canvas { monitors }, start_ms: 1, fit, computers: vec!["pc".into()],
            pause_on_battery: false, paused: false, picture: false, asset_hash: None,
        } }
    }
    #[test]
    fn stretch_crops_share_one_canvas_including_negative_coordinates() {
        let config = config(one_mk_wallpaper::Fit::Stretch);
        let left = mapping(&config, &config.plan.canvas.monitors[0], (2, 4), (4, 2)).unwrap();
        let right = mapping(&config, &config.plan.canvas.monitors[1], (2, 4), (4, 2)).unwrap();
        assert_eq!(left.columns, [Some(0), Some(4)]); assert_eq!(right.columns, [Some(8), Some(12)]);
        assert_eq!(left.rows, [Some(0), Some(0), Some(16), Some(16)]);
    }
    #[test]
    fn fit_letterboxes_and_fill_crops_without_per_output_recentering() {
        let config = config(one_mk_wallpaper::Fit::Fit);
        let left = mapping(&config, &config.plan.canvas.monitors[0], (2, 4), (4, 2)).unwrap();
        assert_eq!(left.columns, [Some(0), Some(4)]); assert_eq!(left.rows, [None, Some(0), Some(16), None]);
        let config = self::config(one_mk_wallpaper::Fit::Fill);
        let left = mapping(&config, &config.plan.canvas.monitors[0], (2, 4), (4, 2)).unwrap();
        let right = mapping(&config, &config.plan.canvas.monitors[1], (2, 4), (4, 2)).unwrap();
        assert_eq!(left.columns, [Some(4), Some(4)]); assert_eq!(right.columns, [Some(8), Some(8)]);
        assert!(left.rows.iter().all(Option::is_some));
    }
    #[test]
    fn scaling_samples_pixel_centers_without_moving_canvas_seams() {
        let config = config(one_mk_wallpaper::Fit::Stretch);
        let left = mapping(&config, &config.plan.canvas.monitors[0], (4, 8), (4, 2)).unwrap();
        let right = mapping(&config, &config.plan.canvas.monitors[1], (4, 8), (4, 2)).unwrap();
        assert_eq!(left.columns, [Some(0), Some(0), Some(4), Some(4)]);
        assert_eq!(right.columns, [Some(8), Some(8), Some(12), Some(12)]);
    }
}
