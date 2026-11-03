use super::*;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use one_mk_wallpaper::wayland::{GNOME_NAME, GNOME_PATH};

pub(super) struct Bridge {
    connection: zbus::blocking::Connection,
    stopped: Arc<AtomicBool>,
    pub secret: String,
}

impl Bridge {
    pub fn start() -> Result<Self> {
        let connection = one_mk_wallpaper::wayland::gnome_connection().context(one_mk_wallpaper::GNOME_PLAYBACK_UNAVAILABLE)?;
        let mut random = [0u8; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let secret: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).sender("org.freedesktop.DBus")?
            .interface("org.freedesktop.DBus")?.member("NameOwnerChanged")?.add_arg(GNOME_NAME)?.build();
        let mut events = zbus::blocking::MessageIterator::for_match_rule(rule, &connection, Some(4))?;
        let stopped = Arc::new(AtomicBool::new(false));
        let signal = stopped.clone();
        std::thread::spawn(move || {
            let _ = events.next();
            signal.store(true, Ordering::Release); wake();
        });
        let bridge = Self { connection, stopped, secret };
        bridge.proxy()?.call::<_, _, ()>("Register", &(&bridge.secret,))?;
        Ok(bridge)
    }

    fn proxy(&self) -> Result<zbus::blocking::Proxy<'_>> {
        Ok(zbus::blocking::Proxy::new(&self.connection, GNOME_NAME, GNOME_PATH, GNOME_NAME)?)
    }

    pub fn alive(&self) -> bool { !self.stopped.load(Ordering::Acquire) }
    pub fn presented(&self) -> Result<Vec<String>> { Ok(self.proxy()?.call("Presented", &())?) }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        if self.alive() { let _ = self.proxy().and_then(|proxy| Ok(proxy.call::<_, _, ()>("Stop", &())?)); }
        let _ = self.connection.clone().close();
    }
}
