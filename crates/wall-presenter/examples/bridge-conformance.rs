// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(target_os = "linux")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> one_mk_wall::gnome_bridge::Result<()> {
    use one_mk_wall_sync::bridge::{self, State};
    let descriptor = std::env::var("WAYLAND_SOCKET")?.parse()?;
    let mut client = one_mk_wall::gnome_bridge::Bridge::connect(descriptor).await?;
    let proxy = zbus::Proxy::new(client.connection(), client.owner(), bridge::PATH, bridge::NAME).await?;
    for (method, value, expected) in [("Hello", 2_u32, "NotSupported"), ("SetState", 5, "InvalidArgs")] {
        let error = proxy.call_method(method, &(value,)).await.unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
    assert!(proxy.call_method("SetState", &("wrong",)).await.is_err());
    assert!(proxy.call_method("FrameReady", &()).await.is_err());
    client.set_state(State::Starting).await?;
    client.set_state(State::Playing).await?;
    client.set_state(State::Playing).await?;
    client.set_state(State::Paused).await?;
    {
        use futures_util::StreamExt;
        let observer = zbus::Connection::session().await?;
        let observer_proxy = zbus::Proxy::new(&observer, bridge::NAME, bridge::PATH, bridge::NAME).await?;
        let mut observed = observer_proxy.receive_all_signals().await?;
        let proxy = zbus::Proxy::new(client.connection(), client.owner(), bridge::PATH, bridge::NAME).await?;
        proxy.call_method("Hello", &(1_u32,)).await?;
        assert!(tokio::time::timeout(std::time::Duration::from_millis(200), observed.next()).await.is_err(),
            "renderer signals were broadcast to an unbound connection");
    }
    assert!(matches!(client.next().await?, one_mk_wall::gnome_bridge::Event::Pause(0)));
    println!("production bridge awaiting layout");
    assert!(matches!(tokio::time::timeout(std::time::Duration::from_secs(5), client.next()).await??,
        one_mk_wall::gnome_bridge::Event::Layout(_, _)));
    println!("production bridge awaiting reasons");
    assert!(matches!(tokio::time::timeout(std::time::Duration::from_secs(5), client.next()).await??,
        one_mk_wall::gnome_bridge::Event::Pause(15)));
    assert!(tokio::time::timeout(std::time::Duration::from_secs(5), client.next()).await?.is_err());
    let proxy = zbus::Proxy::new(client.connection(), client.owner(), bridge::PATH, bridge::NAME).await?;
    let mut limited = false;
    for index in 0_u32..12 {
        if let Err(error) = proxy.call_method("SetState", &(index % 2,)).await {
            assert!(error.to_string().contains("InvalidArgs"), "{error}");
            limited = true;
            break;
        }
    }
    assert!(limited, "extension did not enforce 10 state changes/second");
    println!("production bridge conformance: authenticated Hello, exact signatures, bounds, state, negatives PASS");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {}
