//! Explicit operator-only synthetic ingestion verification. No app data accessed.
fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() != Some("--confirm-synthetic") {
        anyhow::bail!("Use --confirm-synthetic only after selecting an approved PostHog test project. This sends two synthetic events.");
    }
    let home = tempfile::tempdir()?;
    one_mk_ctl::analytics::set_enabled(home.path(), true)?;
    let result = one_mk_ctl::analytics::verify_synthetic(home.path());
    one_mk_ctl::analytics::set_enabled(home.path(), false)?;
    for (uuid, acknowledgment) in result? {
        println!("uuid={uuid} transport={acknowledgment}");
    }
    println!("Query both UUIDs and the grouped exception in the chosen project. HTTP acceptance is not end-to-end verification.");
    Ok(())
}
