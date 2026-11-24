use super::{Action, Permission, Shared, State};
use futures_util::StreamExt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;
use zbus::{Connection, MatchRule, MessageStream, Proxy, message::Type, proxy::CacheProperties, zvariant::OwnedObjectPath};

const NM: &str = "org.freedesktop.NetworkManager";
const PATH: &str = "/org/freedesktop/NetworkManager";

/// First and longest wait before the watcher tries NetworkManager again after it fails.
const RETRY: (Duration, Duration) = (Duration::from_secs(1), Duration::from_secs(30));

type Task<'a, T> = Pin<Box<dyn Future<Output = zbus::Result<T>> + 'a>>;

pub(super) fn start(shared: Arc<Shared>) -> Box<dyn Fn(Action) + Send + Sync> {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime.block_on(supervise(&shared, receiver, RETRY,
                || Box::pin(watch(&shared)),
                || Box::pin(async { current(&Connection::system().await?).await }))),
            Err(_) => shared.publish(State { unavailable: true, ..State::default() }),
        }
    });
    Box::new(move |action| { let _ = sender.send(action); })
}

/// Keeps `watch` running for as long as the window lives. NetworkManager not being there yet, or
/// restarting, or a bus error, only makes names unavailable for a while: the watcher tries again
/// (at once on a refresh, otherwise after a growing pause) and a refresh in between still reads
/// the current state. Ends when told to stop or when every sender is gone.
async fn supervise<'a, W, C>(shared: &'a Shared, mut actions: UnboundedReceiver<Action>, retry: (Duration, Duration), mut watch: W, mut snapshot: C)
where
    W: FnMut() -> Task<'a, ()>,
    C: FnMut() -> Task<'a, State>,
{
    let mut pause = retry.0;
    loop {
        let started = Instant::now();
        let mut session = watch();
        loop {
            tokio::select! {
                _ = &mut session => break,
                action = actions.recv() => match action {
                    Some(Action::Stop) | None => return,
                    Some(Action::Refresh) => if let Ok(state) = snapshot().await { shared.publish(state); },
                    _ => {}
                }
            }
        }
        drop(session);
        shared.publish(State { unavailable: true, ..State::default() });
        if started.elapsed() >= retry.1 { pause = retry.0; }
        let wait = tokio::time::sleep(pause);
        tokio::pin!(wait);
        pause = (pause * 2).min(retry.1);
        loop {
            tokio::select! {
                _ = &mut wait => break,
                action = actions.recv() => match action {
                    Some(Action::Stop) | None => return,
                    Some(Action::Refresh) => break,
                    _ => {}
                }
            }
        }
    }
}

async fn proxy<'a>(connection: &Connection, path: OwnedObjectPath, interface: &'a str, cache: CacheProperties) -> zbus::Result<Proxy<'a>> {
    zbus::proxy::Builder::new(connection).destination(NM)?.path(path)?.interface(interface)?
        .cache_properties(cache).build().await
}

/// Completes when the bus connection is gone (the bus restarted or was shut down). The property
/// streams below only wait for events from a cache, so they never notice: the connection's own
/// message stream is the one that ends (after an error) when its socket does.
async fn disconnected(connection: &Connection) -> zbus::Result<()> {
    let rule = MatchRule::builder().msg_type(Type::Signal).sender("org.freedesktop.DBus")?.interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?.arg(0, NM)?.build();
    let mut messages = MessageStream::for_match_rule(rule, connection, Some(8)).await?;
    while let Some(Ok(_)) = messages.next().await {}
    Ok(())
}

async fn watch(shared: &Shared) -> zbus::Result<()> {
    let connection = Connection::system().await?;
    let gone = disconnected(&connection);
    tokio::pin!(gone);
    // Property change notifications need the properties cached; reads in `current` stay fresh.
    let manager = proxy(&connection, OwnedObjectPath::try_from(PATH)?, NM, CacheProperties::Lazily).await?;
    let mut active = manager.receive_property_changed::<Vec<OwnedObjectPath>>("ActiveConnections").await;
    let mut primary = manager.receive_property_changed::<OwnedObjectPath>("PrimaryConnection").await;
    shared.publish(current(&connection).await?);
    loop {
        let changed = tokio::select! {
            event = active.next() => event.is_some(),
            event = primary.next() => event.is_some(),
            _ = &mut gone => false,
        };
        if !changed { return Ok(()); }
        shared.publish(current(&connection).await?);
    }
}

async fn current(connection: &Connection) -> zbus::Result<State> {
    let manager = proxy(connection, OwnedObjectPath::try_from(PATH)?, NM, CacheProperties::No).await?;
    let active: Vec<OwnedObjectPath> = manager.get_property("ActiveConnections").await?;
    let mut state = State { permission: Permission::Allowed, ..State::default() };
    for path in active {
        // A connection can go away between the list and its details: that one is not there any more.
        match ssid(connection, path).await {
            Ok(Some(ssid)) => state.ssids.push(ssid),
            Ok(None) => {}
            Err(zbus::Error::MethodError(..)) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(state)
}

async fn ssid(connection: &Connection, path: OwnedObjectPath) -> zbus::Result<Option<String>> {
    let connection_proxy = proxy(connection, path, "org.freedesktop.NetworkManager.Connection.Active", CacheProperties::No).await?;
    let kind: String = connection_proxy.get_property("Type").await?;
    if kind != "802-11-wireless" { return Ok(None); }
    let access_point: OwnedObjectPath = connection_proxy.get_property("SpecificObject").await?;
    if access_point.as_str() == "/" { return Ok(None); }
    let access_point = proxy(connection, access_point, "org.freedesktop.NetworkManager.AccessPoint", CacheProperties::No).await?;
    let ssid: Vec<u8> = access_point.get_property("Ssid").await?;
    Ok((!ssid.is_empty()).then(|| String::from_utf8_lossy(&ssid).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const FAST: (Duration, Duration) = (Duration::from_millis(5), Duration::from_millis(20));

    fn shared() -> Arc<Shared> {
        Arc::new(Shared { state: Mutex::new(State::default()), changed: Box::new(|_| {}) })
    }
    fn allowed(name: &str) -> State { State { ssids: vec![name.into()], permission: Permission::Allowed, ..State::default() } }
    fn until(shared: &Shared, mut done: impl FnMut(&State) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if done(&shared.state.lock().unwrap()) { return true; }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    #[test]
    fn a_failed_start_is_retried_and_the_names_come_back() {
        let shared = shared();
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let tries = Arc::new(AtomicUsize::new(0));
        let worker = {
            let (shared, tries) = (shared.clone(), tries.clone());
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                runtime.block_on(supervise(&shared, receiver, FAST,
                    || {
                        let (shared, tries) = (&shared, tries.clone());
                        Box::pin(async move {
                            // NetworkManager is not there on the first two tries.
                            if tries.fetch_add(1, Ordering::SeqCst) < 2 { return Err(zbus::Error::Failure("no NetworkManager yet".into())); }
                            shared.publish(allowed("Home"));
                            std::future::pending().await
                        })
                    },
                    || Box::pin(async { Ok(allowed("Home")) })));
            })
        };
        assert!(until(&shared, |state| state.ssids == ["Home"] && !state.unavailable), "the watcher never recovered");
        sender.send(Action::Stop).unwrap();
        worker.join().unwrap();
        assert!(tries.load(Ordering::SeqCst) >= 3);
    }

    /// A real connection drop, not an injected error: a private bus is started and then killed.
    #[test]
    fn a_bus_that_goes_away_ends_the_wait() {
        use std::io::BufRead;
        let Ok(mut bus) = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn()
        else {
            eprintln!("no dbus-daemon here; skipped");
            return;
        };
        let mut address = String::new();
        std::io::BufReader::new(bus.stdout.take().unwrap()).read_line(&mut address).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let connection = zbus::connection::Builder::address(address.trim()).unwrap().build().await.unwrap();
            let wait = disconnected(&connection);
            tokio::pin!(wait);
            // Alive: still waiting.
            assert!(tokio::time::timeout(Duration::from_millis(200), &mut wait).await.is_err());
            bus.kill().unwrap();
            bus.wait().unwrap();
            assert!(tokio::time::timeout(Duration::from_secs(5), &mut wait).await.is_ok(), "the lost bus was not noticed");
        });
    }

    #[test]
    fn a_refresh_reads_the_current_state_while_the_watcher_is_down() {
        let shared = shared();
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let worker = {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                // The watcher stays up (nothing to say); only refreshes read the state.
                runtime.block_on(supervise(&shared, receiver, FAST,
                    || Box::pin(std::future::pending()),
                    || Box::pin(async { Ok(allowed("Office")) })));
            })
        };
        sender.send(Action::Refresh).unwrap();
        assert!(until(&shared, |state| state.ssids == ["Office"]));
        sender.send(Action::Stop).unwrap();
        worker.join().unwrap();
    }
}
