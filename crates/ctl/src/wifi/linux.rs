use super::{Action, Permission, Shared, State};
use futures_util::StreamExt;
use std::sync::Arc;
use zbus::{Connection, Proxy, proxy::CacheProperties, zvariant::OwnedObjectPath};

const NM: &str = "org.freedesktop.NetworkManager";
const PATH: &str = "/org/freedesktop/NetworkManager";

pub(super) fn start(shared: Arc<Shared>) -> Box<dyn Fn(Action) + Send + Sync> {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread().enable_all().build().map(|runtime| {
            runtime.block_on(async {
                let watching = watch(&shared);
                tokio::pin!(watching);
                loop {
                    tokio::select! {
                        result = &mut watching => return result,
                        action = receiver.recv() => match action {
                            Some(Action::Stop) | None => return Ok(()),
                            Some(Action::Refresh) => {
                                if let Ok(connection) = Connection::system().await {
                                    shared.publish(current(&connection).await?);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            })
        });
        if !matches!(result, Ok(Ok(()))) {
            shared.publish(State { unavailable: true, ..State::default() });
        }
    });
    Box::new(move |action| { let _ = sender.send(action); })
}

async fn proxy<'a>(connection: &Connection, path: OwnedObjectPath, interface: &'a str) -> zbus::Result<Proxy<'a>> {
    zbus::proxy::Builder::new(connection).destination(NM)?.path(path)?.interface(interface)?
        .cache_properties(CacheProperties::No).build().await
}

async fn watch(shared: &Shared) -> zbus::Result<()> {
    let connection = Connection::system().await?;
    let manager = proxy(&connection, OwnedObjectPath::try_from(PATH)?, NM).await?;
    let mut active = manager.receive_property_changed::<Vec<OwnedObjectPath>>("ActiveConnections").await;
    let mut primary = manager.receive_property_changed::<OwnedObjectPath>("PrimaryConnection").await;
    shared.publish(current(&connection).await?);
    loop {
        let changed = tokio::select! {
            event = active.next() => event.is_some(),
            event = primary.next() => event.is_some(),
        };
        if !changed { return Ok(()); }
        shared.publish(current(&connection).await?);
    }
}

async fn current(connection: &Connection) -> zbus::Result<State> {
    let manager = proxy(connection, OwnedObjectPath::try_from(PATH)?, NM).await?;
    let active: Vec<OwnedObjectPath> = manager.get_property("ActiveConnections").await?;
    let mut state = State { permission: Permission::Allowed, ..State::default() };
    for path in active {
        let connection_proxy = proxy(connection, path, "org.freedesktop.NetworkManager.Connection.Active").await?;
        let kind: String = connection_proxy.get_property("Type").await?;
        if kind != "802-11-wireless" { continue; }
        let access_point: OwnedObjectPath = connection_proxy.get_property("SpecificObject").await?;
        if access_point.as_str() == "/" { continue; }
        let access_point = proxy(connection, access_point, "org.freedesktop.NetworkManager.AccessPoint").await?;
        let ssid: Vec<u8> = access_point.get_property("Ssid").await?;
        if !ssid.is_empty() { state.ssids.push(String::from_utf8_lossy(&ssid).into_owned()); }
    }
    Ok(state)
}
