#[cfg(target_os = "linux")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> one_mk_wall::gnome_bridge::Result<()> {
    let descriptor = std::env::var("WAYLAND_SOCKET")?.parse()?;
    let mut client = one_mk_wall::gnome_bridge::Bridge::connect(descriptor).await?;
    client.set_state(one_mk_wall_sync::bridge::State::Paused).await?;
    while client.next().await.is_ok() {}
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {}
