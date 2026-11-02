#![cfg(target_os = "linux")]

use one_mk_ctl::client::{self, Request};
use std::{process::{Command, Stdio}, time::Duration};

#[test]
fn wallpaper_off_has_no_idle_cpu_ticks() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("settings.toml"), "[update]\ncheck = false\n").unwrap();
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_one-mk"))
        .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", directory.path())
        .args(["--home", directory.path().to_str().unwrap(), "run", "--backend", "null", "--no-discovery", "--no-clipboard", "--no-notify", "--port", "0"])
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let result = std::panic::catch_unwind(|| {
        let mut ready = false;
        for _ in 0..100 {
            if client::request(directory.path(), Request::Status, Duration::from_secs(1)).ok().flatten().is_some() { ready = true; break; }
            std::thread::sleep(Duration::from_millis(30));
        }
        assert!(ready, "daemon failed to start");
        std::thread::sleep(Duration::from_millis(500));
        let ticks = || {
            let stat = std::fs::read_to_string(format!("/proc/{}/stat", daemon.id())).unwrap();
            let fields: Vec<_> = stat.rsplit_once(')').unwrap().1.split_whitespace().collect();
            fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
        };
        let before = ticks();
        std::thread::sleep(Duration::from_secs(2));
        assert_eq!(ticks() - before, 0, "wallpaper off must not add polling or timers");
        assert!(!directory.path().join("wallpaper-video/player.json").exists());
    });
    let _ = daemon.kill();
    let _ = daemon.wait();
    if let Err(error) = result { std::panic::resume_unwind(error); }
}
