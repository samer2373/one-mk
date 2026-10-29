//! The clipboard and notification commands, on both sides of the control
//! channel: what `one-mk clipboard`, `clip-send`, `clip-get` and `notify`
//! do in the command, and how the running daemon answers them (including the
//! progress `one-mk send` reports). `app.rs` only forwards requests here.

use crate::app::Services;
use crate::control::{self, Request};
use crate::notify;
use crate::settings::{ClipSettings, Settings};
use anyhow::{Result, bail};
use one_mk_net::{Fingerprint, TrustStore};
use one_mk_proto::ClipAck;
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;
use tokio::sync::oneshot;

/// `on` or `off`, for the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnOff(pub bool);

impl FromStr for OnOff {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "on" => Ok(Self(true)),
            "off" => Ok(Self(false)),
            _ => Err(format!("expected on or off, not {s:?}")),
        }
    }
}

// ------------------------------------------------------------ the daemon

/// A paired computer that is connected right now.
pub struct Online {
    pub fp: Fingerprint,
    pub name: String,
    pub addr: SocketAddr,
}

/// Reads `settings.toml` and applies it to the running services.
pub fn apply_settings(dir: &Path, services: &Services) -> Result<Settings> {
    let s = Settings::load(dir)?;
    apply(&s, services);
    Ok(s)
}

/// Applies the clipboard and notification parts of `s`.
pub fn apply(s: &Settings, services: &Services) {
    if let Some(clip) = &services.clip {
        clip.apply(&s.clipboard);
    }
    services.inbox.transfers().set_notify(s.notify_files && !notify::disabled());
}

/// Answers the requests `app.rs` doesn't handle itself. Never blocks: a
/// clipboard that has to cross the network is answered from a task.
pub fn handle(dir: &Path, services: &Services, online: Vec<Online>, trust: &TrustStore, req: Request, reply: oneshot::Sender<String>) {
    match req {
        Request::Settings => {
            let text = match apply_settings(dir, services) {
                Ok(_) => "ok\n".to_owned(),
                Err(e) => format!("error {e:#}\n"),
            };
            let _ = reply.send(text);
        }
        Request::Transfer { t, finished } => {
            services.inbox.transfers().report(t, finished);
            let _ = reply.send("ok\n".to_owned());
        }
        Request::ClipSend(to) => clip_request(services, &online, trust, to, true, reply),
        Request::ClipGet(from) => clip_request(services, &online, trust, from, false, reply),
        Request::Status | Request::Reload | Request::TakeBack | Request::Switch(_) | Request::LockPointer(_) | Request::LockAll => {
            let _ = reply.send("error unexpected request\n".to_owned());
        }
    }
}

fn clip_request(
    services: &Services,
    online: &[Online],
    trust: &TrustStore,
    who: Option<String>,
    send: bool,
    reply: oneshot::Sender<String>,
) {
    let Some(clip) = services.clip.clone() else {
        let _ = reply.send("error the clipboard is not shared on this computer (it was started with --no-clipboard)\n".into());
        return;
    };
    let target = match pick(online, trust, who.as_deref()) {
        Ok(t) => (t.fp, t.name.clone(), t.addr),
        Err(msg) => {
            let _ = reply.send(format!("error {msg}\n"));
            return;
        }
    };
    tokio::spawn(async move {
        let (fp, name, addr) = target;
        let moved = if send { clip.send_now(fp, addr).await } else { clip.pull(fp, addr).await };
        let size = |m: &crate::clip::Moved| crate::clip::mb(m.bytes);
        let text = match moved {
            Ok(m) => match m.ack {
                Some(ClipAck::FormatsOff) => format!("error {name} has {} switched off, so it ignored your clipboard (see `one-mk clipboard` there)\n", m.what),
                Some(ClipAck::TooLarge) => format!("error your clipboard ({}) is over {name}'s size limit, so it ignored it\n", size(&m)),
                // name, what and size.
                _ => format!("ok\n{name}\n{}\n{}\n", m.what, size(&m)),
            },
            Err(e) => format!("error {e:#}\n"),
        };
        let _ = reply.send(text);
    });
}

/// Finds the computer a request is for among the ones connected.
fn pick<'a>(online: &'a [Online], trust: &TrustStore, who: Option<&str>) -> Result<&'a Online, String> {
    if let Some(q) = who {
        let Some(device) = trust.find(q) else {
            return Err(format!("no paired computer called {q:?} (see `one-mk devices`)"));
        };
        let fp = device.fingerprint();
        return online
            .iter()
            .find(|o| Some(o.fp) == fp)
            .ok_or_else(|| format!("{} is not connected right now", one_mk_proto::sanitize_name(&device.name)));
    }
    match online {
        [] => Err("no computer is connected right now".to_owned()),
        [only] => Ok(only),
        many => {
            let names: Vec<&str> = many.iter().map(|o| o.name.as_str()).collect();
            Err(format!("several computers are connected ({}); choose one by name", names.join(", ")))
        }
    }
}

// ----------------------------------------------------------- the command

/// What `one-mk clipboard` was asked to change.
#[derive(Default)]
pub struct ClipChange {
    pub text: Option<OnOff>,
    pub html: Option<OnOff>,
    pub image: Option<OnOff>,
    pub max_mb: Option<u32>,
    pub skip_sensitive: Option<OnOff>,
    pub allow_fetch: Option<OnOff>,
}

impl ClipChange {
    fn is_empty(&self) -> bool {
        self.text.is_none()
            && self.html.is_none()
            && self.image.is_none()
            && self.max_mb.is_none()
            && self.skip_sensitive.is_none()
            && self.allow_fetch.is_none()
    }

    pub fn apply(&self, c: &mut ClipSettings) -> Result<()> {
        if let Some(mb) = self.max_mb {
            if mb == 0 || mb > ClipSettings::MAX_MB_LIMIT {
                bail!("--max-mb must be between 1 and {}", ClipSettings::MAX_MB_LIMIT);
            }
            c.max_mb = mb;
        }
        c.text = self.text.map_or(c.text, |v| v.0);
        c.html = self.html.map_or(c.html, |v| v.0);
        c.image = self.image.map_or(c.image, |v| v.0);
        c.skip_sensitive = self.skip_sensitive.map_or(c.skip_sensitive, |v| v.0);
        c.allow_pull = self.allow_fetch.map_or(c.allow_pull, |v| v.0);
        Ok(())
    }
}

/// `one-mk clipboard`: shows the settings, or changes them and applies
/// the change to the running daemon.
pub async fn clipboard(dir: &Path, change: ClipChange) -> Result<()> {
    if change.is_empty() {
        let s = Settings::load(dir)?;
        println!("Clipboard sharing: {}.", s.clipboard.summary());
        println!("Change it with `one-mk clipboard --text|--html|--image on|off --max-mb N`.");
        return Ok(());
    }
    let summary = Settings::update(dir, |s| {
        change.apply(&mut s.clipboard)?;
        Ok(s.clipboard.summary())
    })?;
    println!("Clipboard sharing: {summary}.");
    applied(dir).await
}

/// `one-mk notify [on|off]`: whether a received transfer raises a desktop notification.
pub async fn notify(dir: &Path, state: Option<OnOff>) -> Result<()> {
    let Some(OnOff(on)) = state else {
        let s = Settings::load(dir)?;
        println!("A notification when files arrive: {}.", if s.notify_files { "on" } else { "off" });
        return Ok(());
    };
    Settings::update(dir, |s| {
        s.notify_files = on;
        Ok(())
    })?;
    println!("A notification when files arrive: {}.", if on { "on" } else { "off" });
    applied(dir).await
}

async fn applied(dir: &Path) -> Result<()> {
    match control::request(dir, Request::Settings).await? {
        None => println!("one-mk is not running; this takes effect the next time it starts."),
        Some(r) if r.starts_with("ok") => println!("Applied right away."),
        Some(r) => bail!("one-mk could not apply it: {}", r.trim().trim_start_matches("error ")),
    }
    Ok(())
}

/// `one-mk clip-send` and `clip-get`: asks the daemon to move the clipboard.
pub async fn move_clipboard(dir: &Path, who: Option<String>, send: bool) -> Result<()> {
    let req = if send { Request::ClipSend(who) } else { Request::ClipGet(who) };
    let Some(reply) = control::request(dir, req).await? else {
        bail!("one-mk is not running; start it with `one-mk run`");
    };
    let mut lines = reply.lines();
    match lines.next() {
        Some("ok") => {
            let mut next = || lines.next().unwrap_or_default();
            let (name, what, size) = (next(), next(), next());
            if send {
                println!("Sent your clipboard ({what}, {size}) to {name}.");
            } else {
                println!("Copied {name}'s clipboard ({what}, {size}) to this computer.");
            }
            Ok(())
        }
        Some(e) if e.starts_with("error ") => bail!("{}", e.trim_start_matches("error ")),
        _ => bail!("unexpected answer from one-mk: {reply:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::Server;
    use crate::transfer::Inbox;
    use one_mk_clipboard::Clipboard;
    use one_mk_ctl::status::TransferStatus;
    use one_mk_net::{Device, Identity, Node};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("one-mk-clipctl-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn node() -> Node {
        Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap()
    }

    fn services(dir: &Path, clipboard: bool) -> Services {
        Services {
            clip: clipboard.then(|| crate::clip::ClipSync::new(Clipboard::memory().0, node())),
            inbox: Inbox::new(dir.join("downloads")),
        }
    }

    fn fp(n: u8) -> Fingerprint {
        Fingerprint([n; 32])
    }

    fn online(n: u8, name: &str) -> Online {
        Online { fp: fp(n), name: name.into(), addr: "127.0.0.1:1".parse().unwrap() }
    }

    fn trust(dir: &Path, devices: &[(u8, &str)]) -> TrustStore {
        let mut t = TrustStore::load(dir).unwrap();
        for (n, name) in devices {
            t.upsert(Device { name: (*name).into(), fingerprint: fp(*n).to_string(), ..Default::default() });
        }
        t
    }

    async fn ask(dir: &Path, services: &Services, online: Vec<Online>, req: Request) -> String {
        let (tx, rx) = oneshot::channel();
        handle(dir, services, online, &trust(dir, &[]), req, tx);
        rx.await.unwrap()
    }

    #[test]
    fn on_and_off_are_parsed_and_nothing_else() {
        assert_eq!("on".parse(), Ok(OnOff(true)));
        assert_eq!("OFF".parse(), Ok(OnOff(false)));
        assert!("maybe".parse::<OnOff>().is_err());
        assert!("".parse::<OnOff>().is_err());
    }

    #[test]
    fn changes_touch_only_what_was_named() {
        let mut c = ClipSettings::default();
        ClipChange { image: Some(OnOff(false)), max_mb: Some(5), ..Default::default() }.apply(&mut c).unwrap();
        assert_eq!(c, ClipSettings { image: false, max_mb: 5, ..Default::default() });
        ClipChange { image: Some(OnOff(true)), allow_fetch: Some(OnOff(false)), ..Default::default() }.apply(&mut c).unwrap();
        assert!(c.image && !c.allow_pull && c.max_mb == 5);
        assert!(ClipChange::default().is_empty());
    }

    #[test]
    fn the_size_limit_must_be_sensible() {
        let mut c = ClipSettings::default();
        assert!(ClipChange { max_mb: Some(0), ..Default::default() }.apply(&mut c).is_err());
        assert!(ClipChange { max_mb: Some(ClipSettings::MAX_MB_LIMIT + 1), ..Default::default() }.apply(&mut c).is_err());
        assert_eq!(c.max_mb, 25, "a refused change changes nothing");
        assert!(ClipChange { max_mb: Some(ClipSettings::MAX_MB_LIMIT), ..Default::default() }.apply(&mut c).is_ok());
    }

    #[test]
    fn the_target_computer_is_found_by_name_or_id_or_as_the_only_one_connected() {
        let dir = temp("pick");
        let t = trust(&dir, &[(1, "desk"), (2, "laptop"), (3, "asleep")]);
        let on = vec![online(1, "desk"), online(2, "laptop")];
        let err = |r: Result<&Online, String>| r.err().unwrap();

        assert!(err(pick(&[], &t, None)).contains("no computer is connected"));
        assert_eq!(pick(&on[..1], &t, None).unwrap().name, "desk");
        let e = err(pick(&on, &t, None));
        assert!(e.contains("several") && e.contains("desk") && e.contains("laptop"), "{e}");

        assert_eq!(pick(&on, &t, Some("laptop")).unwrap().name, "laptop");
        assert_eq!(pick(&on, &t, Some("DESK")).unwrap().name, "desk", "names are matched without case");
        assert_eq!(pick(&on, &t, Some(&fp(2).short())).unwrap().name, "laptop", "ids work too");
        assert!(err(pick(&on, &t, Some("asleep"))).contains("asleep is not connected"));
        assert!(err(pick(&on, &t, Some("nobody"))).contains("no paired computer called \"nobody\""));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_daemon_applies_settings_and_shows_reported_sends() {
        let dir = temp("handle");
        let sv = services(&dir, true);
        let clip = sv.clip.clone().unwrap();
        assert_eq!(clip.filter_for_test(), one_mk_clipboard::Filter::default());

        Settings {
            clipboard: ClipSettings { image: false, max_mb: 3, ..Default::default() },
            notify_files: false,
            ..Default::default()
        }
        .save(&dir)
        .unwrap();
        assert_eq!(ask(&dir, &sv, vec![], Request::Settings).await, "ok\n");
        let f = clip.filter_for_test();
        assert!(!f.image && f.text && f.max_bytes == 3 << 20);
        assert!(!sv.inbox.transfers().notifying());

        // A broken file is reported and changes nothing.
        std::fs::write(dir.join("settings.toml"), "clipboard = [").unwrap();
        let r = ask(&dir, &sv, vec![], Request::Settings).await;
        assert!(r.starts_with("error ") && r.contains("settings.toml"), "{r}");
        assert!(!clip.filter_for_test().image);

        // A `one-mk send` reporting in shows up in the status, until it says it is done.
        let t = TransferStatus { id: 5, sending: true, peer: "desk".into(), files: 2, done: 1, total: 4, rate: 9 };
        assert_eq!(ask(&dir, &sv, vec![], Request::Transfer { t: t.clone(), finished: false }).await, "ok\n");
        assert_eq!(sv.inbox.transfers().snapshot(), vec![t.clone()]);
        ask(&dir, &sv, vec![], Request::Transfer { t, finished: true }).await;
        assert!(sv.inbox.transfers().snapshot().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn clipboard_requests_that_cannot_be_served_say_why() {
        let dir = temp("errors");
        let sv = services(&dir, true);
        let r = ask(&dir, &sv, vec![], Request::ClipSend(None)).await;
        assert_eq!(r, "error no computer is connected right now\n");
        let r = ask(&dir, &sv, vec![online(1, "a"), online(2, "b")], Request::ClipGet(None)).await;
        assert!(r.starts_with("error several computers are connected (a, b)"), "{r}");
        let r = ask(&dir, &services(&dir, false), vec![online(1, "a")], Request::ClipSend(None)).await;
        assert!(r.contains("--no-clipboard"), "{r}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The commands change the file and the running daemon picks it up over
    /// the control channel, as `one-mk layout` does.
    #[tokio::test]
    async fn the_commands_apply_live_through_the_control_channel() {
        let dir = temp("live");
        let sv = services(&dir, true);
        let clip = sv.clip.clone().unwrap();
        let mut server = Server::start(&dir).await.unwrap();
        let daemon = {
            let (dir, sv) = (dir.clone(), sv.clone());
            tokio::spawn(async move {
                while let Some((req, reply)) = server.recv().await {
                    handle(&dir, &sv, vec![], &TrustStore::load(&dir).unwrap(), req, reply);
                }
            })
        };

        clipboard(&dir, ClipChange { html: Some(OnOff(false)), max_mb: Some(7), ..Default::default() }).await.unwrap();
        let f = clip.filter_for_test();
        assert!(!f.html && f.text && f.image && f.max_bytes == 7 << 20, "{f:?}");
        assert_eq!(Settings::load(&dir).unwrap().clipboard.max_mb, 7, "and it is saved for the next start");

        notify(&dir, Some(OnOff(false))).await.unwrap();
        assert!(!sv.inbox.transfers().notifying());
        notify(&dir, Some(OnOff(true))).await.unwrap();
        assert!(sv.inbox.transfers().notifying());

        // The commands that move the clipboard report the daemon's error as the error.
        let e = move_clipboard(&dir, None, true).await.unwrap_err();
        assert!(format!("{e}").contains("no computer is connected"), "{e}");
        let e = move_clipboard(&dir, Some("x".into()), false).await.unwrap_err();
        assert!(format!("{e}").contains("no paired computer called"), "{e}");

        daemon.abort();
        let _ = daemon.await;
        // Without a daemon, they say so (settings are still saved).
        move_clipboard(&dir, None, true).await.unwrap_err();
        clipboard(&dir, ClipChange { text: Some(OnOff(false)), ..Default::default() }).await.unwrap();
        assert!(!Settings::load(&dir).unwrap().clipboard.text);
        let _ = std::fs::remove_dir_all(dir);
    }
}
