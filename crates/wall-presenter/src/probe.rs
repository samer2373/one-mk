use libloading::Library;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::ptr;

fn safe_root(value: &str) -> bool {
    value.starts_with('/')
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"/_-.".contains(&byte))
        && Path::new(value).components().all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
        && !value.split('/').any(|part| part == "." || part == "..")
        && value.ends_with("/wall-m1-session")
}

fn required_environment(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("missing private test environment: {name}"))
}

fn isolated_environment() -> Result<PathBuf, String> {
    let root_text = required_environment("ONE_MK_WALL_RISK_ROOT")?;
    if !safe_root(&root_text) {
        return Err("risk root must be an absolute private wall-m1-session path".into());
    }
    let root = PathBuf::from(&root_text);
    let metadata = std::fs::symlink_metadata(&root).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || metadata.mode() & 0o777 != 0o700 || metadata.uid() != unsafe { libc::getuid() } {
        return Err("risk root must be an owned, nonsymlink directory with mode 0700".into());
    }
    if root.canonicalize().map_err(|error| error.to_string())? != root {
        return Err("risk root must not contain symlinks".into());
    }
    for (name, suffix) in [("HOME", "home"), ("XDG_RUNTIME_DIR", "run"), ("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data")] {
        if required_environment(name)? != format!("{root_text}/{suffix}") {
            return Err(format!("{name} is not inside the private session"));
        }
    }
    if std::env::var_os("DISPLAY").is_some()
        || std::env::var_os("LD_PRELOAD").is_some()
        || std::env::var_os("LD_LIBRARY_PATH").is_some()
        || required_environment("WAYLAND_DISPLAY")? != "spike-wl-0"
        || required_environment("GSETTINGS_BACKEND")? != "memory"
        || required_environment("DBUS_SYSTEM_BUS_ADDRESS")? != format!("unix:path={root_text}/run/system_bus")
        || required_environment("DBUS_SESSION_BUS_ADDRESS")?
            != std::fs::read_to_string(root.join("session-bus")).map_err(|error| error.to_string())?.trim()
    {
        return Err("live/ambiguous desktop or loader environment; refusing before loading media libraries".into());
    }
    let socket: c_int = required_environment("WAYLAND_SOCKET")?.parse().map_err(|_| "invalid inherited Wayland socket")?;
    if socket < 3 {
        return Err("Wayland socket must be inherited from the private Shell".into());
    }
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    if unsafe { libc::getsockopt(socket, libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut credentials as *mut libc::ucred).cast(), &mut length) } != 0 {
        return Err("cannot identify inherited Wayland socket owner".into());
    }
    let command = std::fs::read(format!("/proc/{}/cmdline", credentials.pid)).map_err(|error| error.to_string())?;
    let command_parts: Vec<&[u8]> = command.split(|byte| *byte == 0).collect();
    let environment = std::fs::read(format!("/proc/{}/environ", credentials.pid)).map_err(|error| error.to_string())?;
    let private_runtime = format!("XDG_RUNTIME_DIR={root_text}/run");
    if credentials.uid != unsafe { libc::getuid() }
        || !command_parts.contains(&b"--headless".as_slice())
        || !command_parts.contains(&b"--no-x11".as_slice())
        || !environment.split(|byte| *byte == 0).any(|entry| entry == private_runtime.as_bytes())
    {
        return Err("Wayland socket is not owned by the private headless/no-X11 Shell".into());
    }
    for name in ["GST_PLUGIN_SYSTEM_PATH", "GST_PLUGIN_SYSTEM_PATH_1_0"] {
        if !required_environment(name)?.is_empty() {
            return Err("system GStreamer plugin discovery must be disabled".into());
        }
    }
    for name in ["GST_PLUGIN_PATH", "GST_PLUGIN_PATH_1_0"] {
        if required_environment(name)? != format!("{root_text}/plugins") {
            return Err("only the private risk-probe plugin directory is allowed".into());
        }
    }
    if required_environment("GST_REGISTRY_1_0")? != format!("{root_text}/cache/registry.bin")
        || required_environment("GST_REGISTRY_FORK")? != "no"
    {
        return Err("registry/scanner must remain private".into());
    }
    let hash = fixture_hash()?;
    if required_environment("ONE_MK_WALL_RISK_HASH")? != hash {
        return Err("generated fixture hash mismatch; untrusted import is not supported".into());
    }
    Ok(root)
}

pub fn fixture_hash() -> Result<String, String> {
    let root_text = required_environment("ONE_MK_WALL_RISK_ROOT")?;
    if !safe_root(&root_text) {
        return Err("invalid fixture root".into());
    }
    let fixture = Path::new(&root_text).join("fixture.webm");
    let metadata = std::fs::symlink_metadata(&fixture).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > 8 * 1024 * 1024 || metadata.uid() != unsafe { libc::getuid() } {
        return Err("fixture must be an owned regular generated file of at most 8 MiB".into());
    }
    Ok(blake3::hash(&std::fs::read(fixture).map_err(|error| error.to_string())?).to_hex().to_string())
}

#[repr(C)]
struct GError {
    domain: u32,
    code: c_int,
    message: *mut c_char,
}

type SetState = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
type Unref = unsafe extern "C" fn(*mut c_void);

struct Pipeline {
    pipeline: *mut c_void,
    bus: *mut c_void,
    set_state: SetState,
    unref: Unref,
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        unsafe {
            (self.set_state)(self.pipeline, 1);
            (self.unref)(self.bus);
            (self.unref)(self.pipeline);
        }
    }
}

unsafe fn symbol<Function: Copy>(library: &Library, name: &[u8]) -> Result<Function, String> {
    unsafe { library.get::<Function>(name).map(|symbol| *symbol).map_err(|error| error.to_string()) }
}

pub fn run() -> Result<(), String> {
    let root = isolated_environment()?;
    unsafe { present(&root) }
}

unsafe fn present(root: &Path) -> Result<(), String> {
    unsafe {
        let gstreamer = Library::new("libgstreamer-1.0.so.0").map_err(|error| error.to_string())?;
        let glib = Library::new("libglib-2.0.so.0").map_err(|error| error.to_string())?;
        let init: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char, *mut *mut GError) -> c_int = symbol(&gstreamer, b"gst_init_check\0")?;
        let parse: unsafe extern "C" fn(*const c_char, *mut *mut GError) -> *mut c_void = symbol(&gstreamer, b"gst_parse_launch\0")?;
        let set_state: SetState = symbol(&gstreamer, b"gst_element_set_state\0")?;
        let unref: Unref = symbol(&gstreamer, b"gst_object_unref\0")?;
        let get_bus: unsafe extern "C" fn(*mut c_void) -> *mut c_void = symbol(&gstreamer, b"gst_element_get_bus\0")?;
        let pop: unsafe extern "C" fn(*mut c_void, u64, u32) -> *mut c_void = symbol(&gstreamer, b"gst_bus_timed_pop_filtered\0")?;
        let parse_error: unsafe extern "C" fn(*mut c_void, *mut *mut GError, *mut *mut c_char) = symbol(&gstreamer, b"gst_message_parse_error\0")?;
        let free_error: unsafe extern "C" fn(*mut GError) = symbol(&glib, b"g_error_free\0")?;
        let free: unsafe extern "C" fn(*mut c_void) = symbol(&glib, b"g_free\0")?;
        let unref_message: unsafe extern "C" fn(*mut c_void) = symbol(&gstreamer, b"gst_mini_object_unref\0")?;
        let mut error = ptr::null_mut();
        if init(ptr::null_mut(), ptr::null_mut(), &mut error) == 0 {
            let detail = error_text(error);
            if !error.is_null() { free_error(error); }
            return Err(detail);
        }
        let own_surface = std::env::var("ONE_MK_WALL_RISK_SINK").as_deref() != Ok("wayland");
        let sink = if own_surface { "appsink name=frames sync=true max-buffers=1" } else { "waylandsink sync=true" };
        let description = CString::new(format!("filesrc location=\"{}/fixture.webm\" ! matroskademux ! vp9dec threads=2 ! videoconvert ! video/x-raw,format=BGRx ! {sink}", root.display())).map_err(|error| error.to_string())?;
        let pipeline = parse(description.as_ptr(), &mut error);
        if !error.is_null() || pipeline.is_null() {
            let detail = error_text(error);
            if !error.is_null() { free_error(error); }
            if !pipeline.is_null() { unref(pipeline); }
            return Err(detail);
        }
        let bus = get_bus(pipeline);
        if bus.is_null() {
            unref(pipeline);
            return Err("pipeline has no bus".into());
        }
        let pipeline = Pipeline { pipeline, bus, set_state, unref };
        if set_state(pipeline.pipeline, 4) == 0 {
            return Err("GStreamer refused PLAYING".into());
        }
        println!("risk probe: PLAYING requested; first damage must be measured by the isolated Shell, not this log");
        if own_surface {
            return crate::surface::present(root, &gstreamer, pipeline.pipeline);
        }
        let message = pop(pipeline.bus, 45_000_000_000, 2);
        if !message.is_null() {
            let mut debug = ptr::null_mut();
            parse_error(message, &mut error, &mut debug);
            let detail = error_text(error);
            if !error.is_null() { free_error(error); }
            if !debug.is_null() { free(debug.cast()); }
            unref_message(message);
            return Err(detail);
        }
        Ok(())
    }
}

unsafe fn error_text(error: *mut GError) -> String {
    if error.is_null() {
        return "GStreamer failure without details".into();
    }
    unsafe { CStr::from_ptr((*error).message).to_string_lossy().into_owned() }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unsafe_root_names_are_rejected() {
        assert!(!super::safe_root("/tmp/a;touch /home/owner/secret"));
        assert!(!super::safe_root("/tmp/a\"b"));
        assert!(!super::safe_root("relative"));
        assert!(!super::safe_root("/work/tree/../wall-m1-session"));
        assert!(!super::safe_root("/work/tree/wall-m1-session/"));
        assert!(super::safe_root("/work/tree/target/wall-m1-session"));
    }
}
