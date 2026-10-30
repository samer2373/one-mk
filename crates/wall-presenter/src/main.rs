#[cfg(all(target_os = "linux", feature = "gnome-risk-probe"))]
mod probe;
#[cfg(all(target_os = "linux", feature = "gnome-risk-probe"))]
mod surface;
#[cfg(all(target_os = "linux", feature = "gnome-risk-probe"))]
mod bridge;

fn main() -> std::process::ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments == ["--help"] {
        println!("one-mk-wall: optional wallpaper component, default OFF\n\
                  Usage: one-mk-wall --help | status | doctor\n\
                  Import and playback are unavailable: sandbox/package qualification is incomplete.\n\
                  This binary is not started by the daemon and never installs a GNOME extension.");
        return std::process::ExitCode::SUCCESS;
    }
    if arguments == ["--present=gnome"] {
        eprintln!("wallpaper stays static: owner-authenticated daemon presentation plan and media pipeline are not yet integrated; no surface or decoder started");
        return std::process::ExitCode::from(3);
    }
    if arguments == ["status"] || arguments == ["doctor"] {
        println!("wallpaper: off; production importer/presenter unavailable; native hardware UNTESTED");
        return if arguments == ["status"] {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(3)
        };
    }
    #[cfg(all(target_os = "linux", feature = "gnome-risk-probe"))]
    if arguments == ["--risk-hash-fixture"] {
        return match probe::fixture_hash() {
            Ok(hash) => { println!("{hash}"); std::process::ExitCode::SUCCESS }
            Err(error) => { eprintln!("fixture hash refused: {error}"); std::process::ExitCode::from(3) }
        };
    }
    #[cfg(all(target_os = "linux", feature = "gnome-risk-probe"))]
    if arguments == ["--present=gnome", "--risk-probe"] {
        return match probe::run() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("wallpaper risk probe refused/failed: {error}");
                std::process::ExitCode::from(3)
            }
        };
    }
    eprintln!("wallpaper unavailable: M1 qualification incomplete; run one-mk-wall doctor. No import or playback started.");
    std::process::ExitCode::from(3)
}
