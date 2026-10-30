use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

static CANCELLED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "linux")]
extern "C" fn cancel(_signal: libc::c_int) {
    CANCELLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn main() -> std::process::ExitCode {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let json = arguments.iter().any(|argument| argument == "--json");
    if arguments == ["--build-info"] {
        println!("{}", serde_json::json!({"test_faults": cfg!(feature = "test-faults"), "schema": 1}));
        return std::process::ExitCode::SUCCESS;
    }
    if arguments.len() == 2 && arguments[0] == "--runtime-inventory" {
        return match one_mk_wall_import::sandbox::runtime_inventory(std::path::Path::new(&arguments[1])) {
            Ok(manifest) => { println!("{}", serde_json::to_string(&manifest).expect("runtime inventory")); std::process::ExitCode::SUCCESS }
            Err(error) => failure(&error, 3, true),
        };
    }
    if arguments.first().is_some_and(|argument| argument == "--decode-webm") {
        let result = (|| {
            if arguments.len() != 4 { return Err("invalid internal decoder invocation".into()); }
            let codec = arguments[1].to_str().ok_or("invalid codec")?;
            let width = arguments[2].to_str().ok_or("invalid width")?.parse().map_err(|_| "invalid width")?;
            let height = arguments[3].to_str().ok_or("invalid height")?.parse().map_err(|_| "invalid height")?;
            one_mk_wall_import::decode_webm(codec, width, height)
        })();
        return worker_result(result);
    }
    if arguments == ["--worker"] {
        return worker_result(one_mk_wall_import::worker());
    }
    if arguments == ["--sandbox-check"] {
        return worker_result(one_mk_wall_import::sandbox::check_worker());
    }
    #[cfg(all(target_os = "linux", feature = "test-faults"))]
    if arguments == ["--sandbox-storage-check"] {
        return worker_result(one_mk_wall_import::sandbox::check_storage_pressure());
    }
    if arguments.iter().any(|argument| argument == "--help") {
        println!("Optional finite wallpaper importer (never starts playback)\nUsage: one-mk wallpaper import FILE --output PRIVATE_DIR --runtime AUDITED_ROOT [--json]\n       one-mk wallpaper doctor [--runtime AUDITED_ROOT] [--json]\nExit: 0 complete/available, 2 usage/validation, 3 sandbox unavailable, 4 path authority refused, 5 operational failure, 130 cancelled\nOnly VP9/AV1 WebM + GIF/APNG/animated WebP. Otherwise: convert first.\nWindows/macOS sandbox: UNTESTED/unimplemented, refuses imports.");
        return std::process::ExitCode::SUCCESS;
    }
    #[cfg(target_os = "linux")]
    unsafe {
        libc::signal(libc::SIGINT, cancel as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, cancel as *const () as libc::sighandler_t);
    }
    let mut runtime = None;
    let mut output = None;
    let mut source = None;
    let Some(operation) = arguments.first() else { return failure("missing command; use --help", 2, json); };
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].to_str() {
            Some("--json") => {}
            Some("--runtime" | "--output") => {
                let Some(value) = arguments.get(index + 1) else { return failure("missing option value", 2, json); };
                let slot = if arguments[index] == "--runtime" { &mut runtime } else { &mut output };
                if slot.is_some() { return failure("duplicate option", 2, json); }
                *slot = Some(PathBuf::from(value));
                index += 1;
            }
            _ if operation == "import" && source.is_none() && !arguments[index].as_encoded_bytes().starts_with(b"-") => source = Some(PathBuf::from(&arguments[index])),
            _ => return failure("unexpected argument; use --help", 2, json),
        }
        index += 1;
    }
    if operation == "doctor" {
        let status = one_mk_wall_import::sandbox::doctor(runtime.as_deref());
        if json { println!("{}", serde_json::to_string(&status).expect("serializable status")); }
        else { println!("Wallpaper import sandbox: {}, {}", if status.available { "available" } else { "unavailable" }, status.reason); }
        return std::process::ExitCode::from(if status.available { 0 } else { 3 });
    }
    #[cfg(all(target_os = "linux", feature = "test-faults"))]
    if operation == "storage-check" {
        let Some(runtime) = runtime else { return failure("storage-check requires --runtime", 2, json); };
        return worker_result(one_mk_wall_import::sandbox::storage_check(&runtime));
    }
    if operation != "import" { return failure("unknown command; use --help", 2, json); }
    let Some(source) = source else { return failure("import requires FILE", 2, json); };
    let Some(output) = output else { return failure("import requires --output (existing private directory)", 2, json); };
    let Some(runtime) = runtime else { return failure("sandbox unavailable: refusing to import: select an audited --runtime", 3, json); };
    match one_mk_wall_import::import(one_mk_wall_import::ImportOptions { source: &source, output: &output, runtime: &runtime, cancel: &CANCELLED, timeout: std::time::Duration::from_secs(600) }) {
        Ok(record) => {
            if json { println!("{}", serde_json::to_string(&record).expect("serializable record")); }
            else { println!("Imported {}: {} frames, {} ms; poster saved. Playback/normalization unqualified.", record.source_blake3, record.media.frames, record.media.duration_ms); }
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            let code = if CANCELLED.load(std::sync::atomic::Ordering::Relaxed) { 130 } else if error.contains("sandbox unavailable") { 3 } else if error.starts_with("validation:") { 2 } else if error.contains("symlink") || error.contains("mode 0700") { 4 } else { 5 };
            failure(&error, code, json)
        }
    }
}

fn worker_result(result: Result<(), String>) -> std::process::ExitCode {
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => { println!("{}", serde_json::json!({"error": error})); std::process::ExitCode::from(4) }
    }
}

fn failure(error: &str, code: u8, json: bool) -> std::process::ExitCode {
    if json { println!("{}", serde_json::json!({"error": error, "exit_code": code})); }
    else { eprintln!("wallpaper import: {}", error.chars().filter(|character| !character.is_control()).collect::<String>()); }
    std::process::ExitCode::from(code)
}
