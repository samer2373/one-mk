#[cfg(target_os = "linux")]
#[test]
fn idle_daemons_do_not_poll() {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-idle-wakeups.py");
    let output = match std::process::Command::new("python3")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .arg(script)
        .arg(env!("CARGO_BIN_EXE_one-mk"))
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("SKIP idle wakeups: python3 is not installed");
            return;
        }
        Err(error) => panic!("starting idle wakeup helper: {error}"),
    };
    eprintln!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(output.status.success(), "idle wakeup regression: {}", String::from_utf8_lossy(&output.stderr));
}
