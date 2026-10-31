use std::ffi::OsString;
use std::process::{Command, ExitCode};

pub fn installed() -> Option<std::path::PathBuf> {
    let filename = if cfg!(windows) { "one-mk-wall-import.exe" } else { "one-mk-wall-import" };
    std::env::current_exe().ok()?.parent().map(|directory| directory.join(filename)).filter(|path| path.is_file())
}

pub fn run(arguments: &[OsString]) -> ExitCode {
    let Some(helper) = installed() else {
        if arguments.iter().any(|argument| argument == "--help" || argument == "-h") {
            print!("{}", include_str!("../../proto/src/wallpaper-import-help.txt"));
            println!("\nThe optional importer package is not installed. No import was attempted.");
            return ExitCode::SUCCESS;
        }
        let message = "sandbox unavailable: refusing to import; install the optional one-mk-wall-import package and an audited runtime; main daemon has no wallpaper/video decoder (clipboard PNG remains)";
        if arguments.iter().any(|argument| argument == "--json") { println!("{}", serde_json::json!({"error": message, "exit_code": 3})); }
        else { eprintln!("Wallpaper import sandbox: unavailable, {message}"); }
        return ExitCode::from(3);
    };
    let mut command = Command::new(helper);
    command.args(arguments).env_clear();
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") { command.env("SystemRoot", system_root); }
    #[cfg(unix)] {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        eprintln!("wallpaper helper unavailable: {error}");
        ExitCode::from(3)
    }
    #[cfg(not(unix))]
    match command.status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(4).try_into().unwrap_or(4)),
        Err(error) => { eprintln!("wallpaper helper unavailable: {error}"); ExitCode::from(3) }
    }
}
