//! Explicit operator-only synthetic ingestion verification. No app data accessed.
fn main() -> anyhow::Result<()> {
    let mode = std::env::args().nth(1);
    if mode.as_deref() == Some("--internal-panic-child") {
        let home = std::path::PathBuf::from(std::env::args().nth(2).ok_or_else(|| anyhow::anyhow!("isolated home required"))?);
        let nonce = std::env::var("ONE_MK_VERIFY_NONCE")?;
        anyhow::ensure!(nonce.len() == 32 && std::fs::read_to_string(home.join(".synthetic-owner"))? == nonce, "disposable child authorization required");
        anyhow::ensure!(std::env::var("ONE_MK_POSTHOG_ENVIRONMENT").as_deref() == Ok("verification"), "verification environment required");
        std::panic::set_hook(Box::new(|_| std::process::abort()));
        one_mk_ctl::analytics::initialize(&home, one_mk_ctl::analytics::Origin::Cli);
        panic!("synthetic verification panic; payload must never be uploaded");
    }
    if mode.as_deref() == Some("--confirm-synthetic-panic") {
        anyhow::ensure!(std::env::var("ONE_MK_POSTHOG_ENVIRONMENT").as_deref() == Ok("verification"), "set ONE_MK_POSTHOG_ENVIRONMENT=verification before the isolated panic test");
        let home = tempfile::tempdir()?;
        let nonce = format!("{:032x}", rand::random::<u128>());
        std::fs::write(home.path().join(".synthetic-owner"), &nonce)?;
        one_mk_ctl::analytics::set_enabled(home.path(), true)?;
        let status = std::process::Command::new(std::env::current_exe()?)
            .arg("--internal-panic-child").arg(home.path()).current_dir(home.path())
            .env_clear().env("HOME", home.path()).env("TMPDIR", home.path())
            .env("ONE_MK_VERIFY_NONCE", nonce).env("ONE_MK_POSTHOG_ENVIRONMENT", "verification")
            .env("ONE_MK_POSTHOG_KEY", std::env::var("ONE_MK_POSTHOG_KEY")?)
            .env("ONE_MK_POSTHOG_HOST", std::env::var("ONE_MK_POSTHOG_HOST")?)
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status()?;
        anyhow::ensure!(!status.success(), "isolated child must terminate");
        #[cfg(unix)] {
            use std::os::unix::process::ExitStatusExt;
            anyhow::ensure!(status.signal() == Some(6), "isolated child must terminate with SIGABRT");
        }
        let result = one_mk_ctl::analytics::verify_synthetic_panic(home.path());
        one_mk_ctl::analytics::set_enabled(home.path(), false)?;
        let (uuid, acknowledgment) = result?;
        println!("verification_run_id={uuid} uuid={uuid} transport={acknowledgment} isolated_child_aborted=true");
        return Ok(());
    }
    if mode.as_deref() != Some("--confirm-synthetic") {
        anyhow::bail!("Use --confirm-synthetic only after selecting an approved PostHog test project. This sends eight synthetic events in environment=verification.");
    }
    let home = tempfile::tempdir()?;
    one_mk_ctl::analytics::set_enabled(home.path(), true)?;
    let result = one_mk_ctl::analytics::verify_synthetic(home.path());
    one_mk_ctl::analytics::set_enabled(home.path(), false)?;
    let (marker, receipts) = result?;
    println!("verification_run_id={marker}");
    for (uuid, acknowledgment) in receipts {
        println!("uuid={uuid} transport={acknowledgment}");
    }
    println!("Query all eight UUIDs and the grouped exception in the chosen project. HTTP acceptance is not end-to-end verification.");
    Ok(())
}
