use crate::Input;
use anyhow::{Context, Result, bail, ensure};
use libloading::Library;
use one_mk_wallpaper::{Correction, Hold, Holds, Media, PlayerConfig, correction, now_ms, position};
use std::ffi::{CString, c_char, c_int, c_uint, c_ulong, c_void};
use std::path::Path;
use std::ptr;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use x11_dl::xlib;

type Object = *mut c_void;

macro_rules! api {
    ($library:expr, $name:literal, $signature:ty) => { *$library.get::<$signature>(concat!($name, "\0").as_bytes())? };
}

struct Gst {
    _core: Library,
    _object: Library,
    _video: Library,
    parse: unsafe extern "C" fn(*const c_char, *mut Object) -> Object,
    state: unsafe extern "C" fn(Object, c_int) -> c_int,
    wait: unsafe extern "C" fn(Object, *mut c_int, *mut c_int, u64) -> c_int,
    bus: unsafe extern "C" fn(Object) -> Object,
    pop: unsafe extern "C" fn(Object, u64, c_uint) -> Object,
    duration: unsafe extern "C" fn(Object, c_int, *mut i64) -> c_int,
    position: unsafe extern "C" fn(Object, c_int, *mut i64) -> c_int,
    seek: unsafe extern "C" fn(Object, f64, c_int, c_uint, c_int, i64, c_int, i64) -> c_int,
    named: unsafe extern "C" fn(Object, *const c_char) -> Object,
    pad: unsafe extern "C" fn(Object, *const c_char) -> Object,
    caps: unsafe extern "C" fn(Object) -> Object,
    structure: unsafe extern "C" fn(Object, c_uint) -> Object,
    integer: unsafe extern "C" fn(Object, *const c_char, *mut c_int) -> c_int,
    overlay: unsafe extern "C" fn(Object, usize),
    unref: unsafe extern "C" fn(Object),
    mini_unref: unsafe extern "C" fn(Object),
    has_name: unsafe extern "C" fn(Object, *const c_char) -> c_int,
    clock: unsafe extern "C" fn() -> Object,
    use_clock: unsafe extern "C" fn(Object, Object),
    internal_time: unsafe extern "C" fn(Object) -> u64,
    clock_time: unsafe extern "C" fn(Object) -> u64,
    calibrate: unsafe extern "C" fn(Object, u64, u64, u64, u64),
    connect: unsafe extern "C" fn(Object, *const c_char, unsafe extern "C" fn(Object, Object, Object, Object), Object, Object, c_uint) -> c_ulong,
    property: unsafe extern "C" fn(Object, *const c_char) -> Object,
    set: unsafe extern "C" fn(Object, *const c_char, ...),
}

impl Gst {
    fn load() -> Result<Self> {
        unsafe {
            let core = Library::new("libgstreamer-1.0.so.0").context("Install GStreamer to play video wallpapers")?;
            let object = Library::new("libgobject-2.0.so.0")?;
            let video = Library::new("libgstvideo-1.0.so.0")?;
            let init = api!(core, "gst_init_check", unsafe extern "C" fn(Object, Object, *mut Object) -> c_int);
            ensure!(init(ptr::null_mut(), ptr::null_mut(), ptr::null_mut()) != 0, "GStreamer could not start");
            let find = api!(core, "gst_element_factory_find", unsafe extern "C" fn(*const c_char) -> Object);
            let rank = api!(core, "gst_plugin_feature_set_rank", unsafe extern "C" fn(Object, c_uint));
            let unref = api!(object, "g_object_unref", unsafe extern "C" fn(Object));
            for codec in ["h264", "h265", "vp8", "vp9", "av1"] {
                for prefix in ["nv", "va", "vaapi"] {
                    let name = CString::new(format!("{prefix}{codec}dec"))?;
                    let factory = find(name.as_ptr());
                    if !factory.is_null() {
                        rank(factory, if prefix != "nv" && Path::new("/dev/dri").exists() { 512 } else { 0 });
                        unref(factory);
                    }
                }
            }
            Ok(Self {
                connect: api!(object, "g_signal_connect_data", unsafe extern "C" fn(Object, *const c_char, unsafe extern "C" fn(Object, Object, Object, Object), Object, Object, c_uint) -> c_ulong),
                property: api!(object, "g_object_class_find_property", unsafe extern "C" fn(Object, *const c_char) -> Object),
                set: api!(object, "g_object_set", unsafe extern "C" fn(Object, *const c_char, ...)),
                clock: api!(core, "gst_system_clock_obtain", unsafe extern "C" fn() -> Object),
                use_clock: api!(core, "gst_pipeline_use_clock", unsafe extern "C" fn(Object, Object)),
                internal_time: api!(core, "gst_clock_get_internal_time", unsafe extern "C" fn(Object) -> u64),
                clock_time: api!(core, "gst_clock_get_time", unsafe extern "C" fn(Object) -> u64),
                calibrate: api!(core, "gst_clock_set_calibration", unsafe extern "C" fn(Object, u64, u64, u64, u64)),
                has_name: api!(core, "gst_message_has_name", unsafe extern "C" fn(Object, *const c_char) -> c_int),
                parse: api!(core, "gst_parse_launch", unsafe extern "C" fn(*const c_char, *mut Object) -> Object),
                state: api!(core, "gst_element_set_state", unsafe extern "C" fn(Object, c_int) -> c_int),
                wait: api!(core, "gst_element_get_state", unsafe extern "C" fn(Object, *mut c_int, *mut c_int, u64) -> c_int),
                bus: api!(core, "gst_element_get_bus", unsafe extern "C" fn(Object) -> Object),
                pop: api!(core, "gst_bus_timed_pop_filtered", unsafe extern "C" fn(Object, u64, c_uint) -> Object),
                duration: api!(core, "gst_element_query_duration", unsafe extern "C" fn(Object, c_int, *mut i64) -> c_int),
                position: api!(core, "gst_element_query_position", unsafe extern "C" fn(Object, c_int, *mut i64) -> c_int),
                seek: api!(core, "gst_element_seek", unsafe extern "C" fn(Object, f64, c_int, c_uint, c_int, i64, c_int, i64) -> c_int),
                named: api!(core, "gst_bin_get_by_name", unsafe extern "C" fn(Object, *const c_char) -> Object),
                pad: api!(core, "gst_element_get_static_pad", unsafe extern "C" fn(Object, *const c_char) -> Object),
                caps: api!(core, "gst_pad_get_current_caps", unsafe extern "C" fn(Object) -> Object),
                structure: api!(core, "gst_caps_get_structure", unsafe extern "C" fn(Object, c_uint) -> Object),
                integer: api!(core, "gst_structure_get_int", unsafe extern "C" fn(Object, *const c_char, *mut c_int) -> c_int),
                overlay: api!(video, "gst_video_overlay_set_window_handle", unsafe extern "C" fn(Object, usize)),
                unref: api!(object, "g_object_unref", unsafe extern "C" fn(Object)),
                mini_unref: api!(core, "gst_mini_object_unref", unsafe extern "C" fn(Object)),
                _core: core, _object: object, _video: video,
            })
        }
    }
    fn pipeline(&self, text: &str) -> Result<Pipeline<'_>> {
        let mut error = ptr::null_mut();
        let pipeline = unsafe { (self.parse)(CString::new(text)?.as_ptr(), &mut error) };
        if !error.is_null() {
            #[repr(C)]
            struct Error { domain: u32, code: c_int, message: *const c_char }
            let detail = unsafe { std::ffi::CStr::from_ptr((error as *const Error).as_ref().unwrap().message) }.to_string_lossy().into_owned();
            if !pipeline.is_null() { unsafe { (self.unref)(pipeline); } }
            let glib = unsafe { Library::new("libglib-2.0.so.0")? };
            let free = unsafe { api!(glib, "g_error_free", unsafe extern "C" fn(Object)) };
            unsafe { free(error); }
            bail!("GStreamer: {detail}");
        }
        ensure!(!pipeline.is_null(), "GStreamer could not open this video");
        let source = self.object(pipeline, "source")?;
        unsafe {
            (self.connect)(source, c"deep-element-added".as_ptr(), decoder_added, (self as *const Self).cast_mut().cast(), ptr::null_mut(), 0);
            (self.unref)(source);
        }
        Ok(Pipeline { gst: self, raw: pipeline })
    }
    fn object(&self, pipeline: Object, name: &str) -> Result<Object> {
        let object = unsafe { (self.named)(pipeline, CString::new(name)?.as_ptr()) };
        ensure!(!object.is_null(), "GStreamer video output is missing");
        Ok(object)
    }
}

unsafe extern "C" fn decoder_added(_: Object, _: Object, element: Object, data: Object) {
    let gst = unsafe { &*data.cast::<Gst>() };
    let class = unsafe { *element.cast::<Object>() };
    for name in [c"threads", c"max-threads"] {
        if !unsafe { (gst.property)(class, name.as_ptr()) }.is_null() {
            unsafe { (gst.set)(element, name.as_ptr(), 2u32, ptr::null::<c_char>()); }
        }
    }
}

struct Pipeline<'a> { gst: &'a Gst, raw: Object }
struct Clock<'a> { gst: &'a Gst, raw: Object }
impl Drop for Clock<'_> { fn drop(&mut self) { unsafe { (self.gst.unref)(self.raw); } } }
impl Clock<'_> {
    fn rate(&self, rate: f64) {
        unsafe {
            let internal = (self.gst.internal_time)(self.raw);
            let external = (self.gst.clock_time)(self.raw);
            (self.gst.calibrate)(self.raw, internal, external, (rate * 1000.0).round() as u64, 1000);
        }
    }
}
impl Drop for Pipeline<'_> {
    fn drop(&mut self) { unsafe { (self.gst.state)(self.raw, 1); (self.gst.unref)(self.raw); } }
}
impl Pipeline<'_> {
    fn preroll(&self) -> Result<()> {
        unsafe {
            ensure!((self.gst.state)(self.raw, 3) != 0, "This video cannot be decoded by the system");
            ensure!((self.gst.wait)(self.raw, ptr::null_mut(), ptr::null_mut(), 15_000_000_000) == 1,
                "The system could not decode this video. Check that GStreamer has a decoder for it.");
        }
        Ok(())
    }
    fn segment(&self, millis: u64, duration: u64, flush: bool) {
        unsafe { (self.gst.seek)(self.raw, 1.0, 3, 12 | u32::from(flush), 1, millis.saturating_mul(1_000_000) as i64, 1, duration.saturating_mul(1_000_000) as i64); }
    }
}

fn quoted(text: &str) -> String { format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"")) }
fn uri(path: &Path) -> Result<String> {
    let path = path.canonicalize()?;
    let mut uri = String::from("file://");
    for byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(byte) { uri.push(char::from(*byte)); }
        else { uri.push_str(&format!("%{byte:02X}")); }
    }
    Ok(uri)
}

pub fn probe(path: &Path, poster: Option<&Path>) -> Result<Media> {
    let gst = Gst::load()?;
    let pipeline = gst.pipeline(&format!("uridecodebin uri={} caps=video/x-raw name=source source. ! queue max-size-bytes=16777216 max-size-time=2000000000 ! videoconvert ! fakesink name=probe", quoted(&uri(path)?)))?;
    pipeline.preroll()?;
    unsafe {
        let sink = gst.object(pipeline.raw, "probe")?;
        let pad = (gst.pad)(sink, c"sink".as_ptr());
        (gst.unref)(sink);
        ensure!(!pad.is_null(), "No video track found");
        let caps = (gst.caps)(pad);
        (gst.unref)(pad);
        ensure!(!caps.is_null(), "No video dimensions found");
        let structure = (gst.structure)(caps, 0);
        let (mut width, mut height, mut duration) = (0, 0, 0);
        (gst.integer)(structure, c"width".as_ptr(), &mut width);
        (gst.integer)(structure, c"height".as_ptr(), &mut height);
        (gst.mini_unref)(caps);
        ensure!((gst.duration)(pipeline.raw, 3, &mut duration) != 0 && duration > 0, "This video has no known duration");
        if let Some(poster) = poster {
            let poster_pipeline = gst.pipeline(&format!("uridecodebin uri={} caps=video/x-raw name=source source. ! queue ! videoconvert ! videoscale ! video/x-raw,width=640,pixel-aspect-ratio=1/1 ! pngenc snapshot=true ! filesink location={}", quoted(&uri(path)?), quoted(&poster.to_string_lossy())))?;
            (gst.state)(poster_pipeline.raw, 4);
            let bus = (gst.bus)(poster_pipeline.raw);
            let message = (gst.pop)(bus, 15_000_000_000, 1 | 2);
            if !message.is_null() { (gst.mini_unref)(message); }
            (gst.unref)(bus);
        }
        Ok(Media { width: width as u32, height: height as u32, duration_ms: duration as u64 / 1_000_000 })
    }
}

struct Desktop {
    xlib: xlib::Xlib,
    display: *mut xlib::Display,
    windows: Vec<c_ulong>,
}
impl Desktop {
    fn property(&self, window: c_ulong, name: &std::ffi::CStr) -> Vec<c_ulong> {
        unsafe {
            let atom = (self.xlib.XInternAtom)(self.display, name.as_ptr(), 0);
            let mut actual = 0;
            let mut format = 0;
            let mut count = 0;
            let mut remaining = 0;
            let mut data = ptr::null_mut();
            let result = (self.xlib.XGetWindowProperty)(self.display, window, atom, 0, 4096, 0, xlib::AnyPropertyType as c_ulong, &mut actual, &mut format, &mut count, &mut remaining, &mut data);
            let values = if result == 0 && format == 32 && !data.is_null() { std::slice::from_raw_parts(data.cast::<c_ulong>(), count as usize).to_vec() } else { Vec::new() };
            if !data.is_null() { (self.xlib.XFree)(data.cast()); }
            values
        }
    }

    fn covered(&self, config: &PlayerConfig) -> bool {
        unsafe {
            let root = (self.xlib.XDefaultRootWindow)(self.display);
            let clients = self.property(root, c"_NET_CLIENT_LIST_STACKING");
            let desktop = (self.xlib.XInternAtom)(self.display, c"_NET_WM_WINDOW_TYPE_DESKTOP".as_ptr(), 0);
            let dock = (self.xlib.XInternAtom)(self.display, c"_NET_WM_WINDOW_TYPE_DOCK".as_ptr(), 0);
            let rectangles: Vec<_> = clients.into_iter().filter(|window| !self.windows.contains(window)).filter_map(|window| {
                let kinds = self.property(window, c"_NET_WM_WINDOW_TYPE");
                if kinds.contains(&desktop) || kinds.contains(&dock) { return None; }
                let mut attributes: xlib::XWindowAttributes = std::mem::zeroed();
                if (self.xlib.XGetWindowAttributes)(self.display, window, &mut attributes) == 0 || attributes.map_state != xlib::IsViewable { return None; }
                let mut x = 0;
                let mut y = 0;
                let mut child = 0;
                if (self.xlib.XTranslateCoordinates)(self.display, window, root, 0, 0, &mut x, &mut y, &mut child) == 0 { return None; }
                Some(one_mk_wallpaper::Rect { x: x.into(), y: y.into(), width: attributes.width.into(), height: attributes.height.into() })
            }).collect();
            config.plan.canvas.monitors.iter().filter(|monitor| monitor.computer == config.computer).all(|monitor| {
                let monitor = monitor.local.unwrap_or(monitor.rect);
                rectangles.iter().any(|window| {
                    let width = ((monitor.x + monitor.width).min(window.x + window.width) - monitor.x.max(window.x)).max(0.0);
                    let height = ((monitor.y + monitor.height).min(window.y + window.height) - monitor.y.max(window.y)).max(0.0);
                    width * height >= monitor.width * monitor.height * 0.25
                })
            })
        }
    }

    fn new(config: &PlayerConfig) -> Result<Self> {
        let xlib = xlib::Xlib::open()?;
        let display = unsafe { (xlib.XOpenDisplay)(ptr::null()) };
        ensure!(!display.is_null(), "Cannot open the X11 desktop");
        unsafe { (xlib.XSetErrorHandler)(Some(ignore_x_error)); }
        let mut desktop = Self { xlib, display, windows: Vec::new() };
        for monitor in config.plan.canvas.monitors.iter().filter(|monitor| monitor.computer == config.computer) {
            let rect = monitor.local.unwrap_or(monitor.rect);
            unsafe {
                let root = (desktop.xlib.XDefaultRootWindow)(display);
                let window = (desktop.xlib.XCreateSimpleWindow)(display, root,
                    rect.x as c_int, rect.y as c_int,
                    (rect.width * monitor.scale) as c_uint, (rect.height * monitor.scale) as c_uint, 0, 0, 0);
                let atom = |name: &std::ffi::CStr| (desktop.xlib.XInternAtom)(display, name.as_ptr(), 0);
                let kind = atom(c"_NET_WM_WINDOW_TYPE_DESKTOP");
                (desktop.xlib.XChangeProperty)(display, window, atom(c"_NET_WM_WINDOW_TYPE"), xlib::XA_ATOM, 32, xlib::PropModeReplace, (&kind as *const c_ulong).cast(), 1);
                let states = [atom(c"_NET_WM_STATE_BELOW"), atom(c"_NET_WM_STATE_STICKY"), atom(c"_NET_WM_STATE_SKIP_TASKBAR"), atom(c"_NET_WM_STATE_SKIP_PAGER")];
                (desktop.xlib.XChangeProperty)(display, window, atom(c"_NET_WM_STATE"), xlib::XA_ATOM, 32, xlib::PropModeReplace, states.as_ptr().cast(), states.len() as c_int);
                let all_desktops: c_ulong = 0xffff_ffff;
                (desktop.xlib.XChangeProperty)(display, window, atom(c"_NET_WM_DESKTOP"), xlib::XA_CARDINAL, 32, xlib::PropModeReplace, (&all_desktops as *const c_ulong).cast(), 1);
                (desktop.xlib.XStoreName)(display, window, c"oneMK Wallpaper".as_ptr());
                (desktop.xlib.XMapWindow)(display, window);
                (desktop.xlib.XLowerWindow)(display, window);
                desktop.windows.push(window);
            }
        }
        unsafe { (desktop.xlib.XSync)(display, 0); }
        Ok(desktop)
    }
}
unsafe extern "C" fn ignore_x_error(_: *mut xlib::Display, _: *mut xlib::XErrorEvent) -> c_int { 0 }
impl Drop for Desktop {
    fn drop(&mut self) { unsafe { for window in &self.windows { (self.xlib.XDestroyWindow)(self.display, *window); } (self.xlib.XCloseDisplay)(self.display); } }
}

fn on_battery() -> bool {
    std::fs::read_dir("/sys/class/power_supply").into_iter().flatten().flatten().any(|entry|
        std::fs::read_to_string(entry.path().join("type")).is_ok_and(|kind| kind.trim() == "Battery")
        && std::fs::read_to_string(entry.path().join("status")).is_ok_and(|state| state.trim() == "Discharging"))
}

pub fn run(mut config: PlayerConfig, receiver: Receiver<Input>) -> Result<()> {
    let desktop = Desktop::new(&config)?;
    ensure!(!desktop.windows.is_empty(), "No monitors selected on this computer");
    let gst = Gst::load()?;
    let mut pipeline_text = format!("uridecodebin uri={} caps=video/x-raw name=source source. ! queue max-size-bytes=16777216 max-size-time=2000000000 ! videoconvert ! tee name=picture ", quoted(&uri(&config.path)?));
    let mut visible = Vec::new();
    for (index, monitor) in config.plan.canvas.monitors.iter().filter(|monitor| monitor.computer == config.computer).enumerate() {
        let slice = config.plan.canvas.slice(monitor, &config.plan.media, config.plan.fit).context("Empty canvas")?;
        if slice.x >= 1.0 || slice.y >= 1.0 || slice.x + slice.width <= 0.0 || slice.y + slice.height <= 0.0 { continue; }
        visible.push(index);
        let width = f64::from(config.plan.media.width);
        let height = f64::from(config.plan.media.height);
        let left = (slice.x * width).round() as i32;
        let top = (slice.y * height).round() as i32;
        let right = (width - (slice.x + slice.width) * width).round() as i32;
        let bottom = (height - (slice.y + slice.height) * height).round() as i32;
        pipeline_text += &format!("picture. ! queue max-size-buffers=2 max-size-bytes=0 max-size-time=0 ! videobox left={left} top={top} right={right} bottom={bottom} fill=black ! videoscale ! video/x-raw,width={},height={},pixel-aspect-ratio=1/1 ! ximagesink name=screen{index} sync=true handle-events=false force-aspect-ratio=false ", (monitor.rect.width * monitor.scale) as u32, (monitor.rect.height * monitor.scale) as u32);
    }
    if visible.is_empty() { pipeline_text += "picture. ! fakesink sync=true"; }
    let pipeline = gst.pipeline(&pipeline_text)?;
    let clock = Clock { gst: &gst, raw: unsafe { (gst.clock)() } };
    unsafe { (gst.use_clock)(pipeline.raw, clock.raw); }
    for (index, window) in desktop.windows.iter().enumerate() {
        if !visible.contains(&index) { continue; }
        let sink = gst.object(pipeline.raw, &format!("screen{index}"))?;
        unsafe { (gst.overlay)(sink, *window as usize); (gst.unref)(sink); }
    }
    pipeline.preroll()?;
    pipeline.segment(position(now_ms(), config.clock_offset_ms, config.plan.start_ms, config.plan.media.duration_ms), config.plan.media.duration_ms, true);
    let bus = unsafe { (gst.bus)(pipeline.raw) };
    let mut holds = Holds::default();
    holds.set(Hold::User, config.plan.paused);
    let mut playing = false;
    let mut last_sync = Instant::now();
    let mut rate = 1.0;
    let mut last_message = "";
    let mut last_power = Instant::now() - Duration::from_secs(5);
    loop {
        match receiver.recv_timeout(Duration::from_millis(if playing { 10 } else { 1000 })) {
            Ok(Input::End) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Input::Offset(offset)) => config.clock_offset_ms = offset,
            _ => {}
        }
        if last_power.elapsed() >= Duration::from_secs(1) {
            holds.set(Hold::Battery, config.plan.pause_on_battery && on_battery());
            holds.set(Hold::Occluded, desktop.covered(&config));
            last_power = Instant::now();
        }
        if holds.message() != last_message { one_mk_wallpaper::report(&config, &holds); last_message = holds.message(); }
        if playing != holds.playing() {
            playing = holds.playing();
            if playing { pipeline.segment(position(now_ms(), config.clock_offset_ms, config.plan.start_ms, config.plan.media.duration_ms), config.plan.media.duration_ms, true); }
            unsafe { (gst.state)(pipeline.raw, if playing { 4 } else { 3 }); }
        }
        if playing && last_sync.elapsed() >= Duration::from_secs(2) {
            let mut actual = 0;
            let expected = position(now_ms(), config.clock_offset_ms, config.plan.start_ms, config.plan.media.duration_ms);
            if unsafe { (gst.position)(pipeline.raw, 3, &mut actual) } != 0 {
                match correction(actual.max(0) as u64 / 1_000_000, expected, config.plan.media.duration_ms) {
                    Correction::Seek(position) => { clock.rate(1.0); pipeline.segment(position, config.plan.media.duration_ms, true); rate = 1.0; }
                    adjustment => {
                        let target = if let Correction::Rate(rate) = adjustment { rate } else { 1.0 };
                        if target != rate { clock.rate(target); rate = target; }
                    }
                }
            }
            last_sync = Instant::now();
        }
        unsafe {
            let message = (gst.pop)(bus, 0, 1 | 2 | (1 << 17));
            if !message.is_null() {
                let error = (gst.has_name)(message, c"GstMessageError".as_ptr()) != 0;
                let segment = (gst.has_name)(message, c"GstMessageSegmentDone".as_ptr()) != 0;
                (gst.mini_unref)(message);
                if error { (gst.unref)(bus); bail!("GStreamer stopped playing this video. Try a video your system can decode."); }
                pipeline.segment(0, config.plan.media.duration_ms, !segment);
            }
            while (desktop.xlib.XPending)(desktop.display) != 0 {
                let mut event = std::mem::zeroed();
                (desktop.xlib.XNextEvent)(desktop.display, &mut event);
            }
        }
    }
    unsafe { (gst.unref)(bus); }
    Ok(())
}
