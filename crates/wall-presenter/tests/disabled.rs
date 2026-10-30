use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

fn invoke(arguments: &[&str]) -> std::process::Output {
    let directory = std::env::temp_dir().join(format!("one-mk-wall-disabled-{}-{}",
        std::process::id(), NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)));
    let builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    builder.create(&directory).expect("create a fresh throwaway environment");
    let output = Command::new(env!("CARGO_BIN_EXE_one-mk-wall"))
        .args(arguments)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &directory)
        .env("XDG_RUNTIME_DIR", &directory)
        .env("ONE_MK_HOME", &directory)
        .output();
    std::fs::remove_dir(directory).expect("finite commands must leave no files behind");
    output.expect("execute optional binary and reap it")
}

#[test]
fn disabled_status_is_a_finite_command() {
    let output = invoke(&["status"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("wallpaper: off"));
}

#[test]
fn unavailable_operations_refuse_without_loading_or_spawning() {
    for arguments in [vec![], vec!["import", "/not/a/media/file"], vec!["--present=gnome"], vec!["--present=gnome", "--risk-probe"]] {
        let output = invoke(&arguments);
        assert_eq!(output.status.code(), Some(3));
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn help_and_doctor_do_not_imply_platform_qualification() {
    let help = invoke(&["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("unavailable"));
    let doctor = invoke(&["doctor"]);
    assert_eq!(doctor.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&doctor.stdout).contains("UNTESTED"));
}
