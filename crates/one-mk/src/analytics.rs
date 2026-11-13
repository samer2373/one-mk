//! Shared analytics, with command-line consent management.
pub use one_mk_ctl::analytics::*;

pub fn command(home: &std::path::Path, action: &str) -> anyhow::Result<()> {
    match action {
        "on" => set_enabled(home, true)?,
        "off" | "reset" => set_enabled(home, false)?,
        "status" => {},
        _ => anyhow::bail!("use analytics on, off, reset or status"),
    }
    println!("Anonymous analytics: {}. PostHog configuration: {}.",
        if identity(home).is_some() { "on" } else { "off" },
        if configured() { "available" } else { "not configured; no events sent" });
    println!("After enabling/resetting, restart the desktop app and daemon. Already-running operations never adopt new consent.");
    Ok(())
}
