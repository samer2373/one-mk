#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    let mut arguments = std::env::args().skip(1);
    if arguments.next().as_deref() != Some("--allow-focused-editor-input") {
        anyhow::bail!("owner-only manual probe: requires --allow-focused-editor-input and a printable test string");
    }
    let text = arguments.next().ok_or_else(|| anyhow::anyhow!("missing test string"))?;
    if arguments.next().is_some() { anyhow::bail!("unexpected argument"); }
    tokio::runtime::Builder::new_current_thread().enable_all().build()?
        .block_on(one_mk_input::portal_typing_probe(&text))?;
    eprintln!("Keysym calls completed; this is NOT evidence that the editor received correct text.");
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("The portal typing probe is Linux-only.");
    std::process::exit(1);
}
