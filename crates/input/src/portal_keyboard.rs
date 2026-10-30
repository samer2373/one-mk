use super::*;
use ashpd::desktop::remote_desktop::KeyState;
use futures::stream::BoxStream;
use one_mk_proto::typing::{character_key_code, printable};

pub(super) struct KeysymKeyboard {
    session: Session<RemoteDesktop>,
    closed: BoxStream<'static, ()>,
    held: Vec<(u32, i32)>,
    pending: Option<i32>,
}

impl KeysymKeyboard {
    pub(super) async fn open(portal: &RemoteDesktop) -> Result<Self> {
        if portal.version() < 1
            || !timeout("keyboard capability", portal.available_device_types()).await?.contains(DeviceType::Keyboard)
        {
            bail!("RemoteDesktop keyboard capability is unavailable");
        }
        let introspection = zbus::fdo::IntrospectableProxy::builder(portal.connection())
            .destination(portal.destination().to_owned())?
            .path(portal.path().to_owned())?
            .build().await?;
        let xml = timeout("keyboard interface", introspection.introspect()).await?;
        if !has_keysym_method(&xml) {
            bail!("RemoteDesktop NotifyKeyboardKeysym is unavailable");
        }
        let session = timeout("creating a keyboard session", portal.create_session(Default::default())).await?;
        let setup = async {
            let bytes = zbus::zvariant::to_bytes(
                zbus::zvariant::serialized::Context::new_dbus(zbus::zvariant::LE, 0), &session,
            )?;
            let (path, _) = bytes.deserialize::<zbus::zvariant::OwnedObjectPath>()?;
            let proxy = zbus::Proxy::new_owned(portal.connection().clone(), portal.destination().to_owned(),
                path, "org.freedesktop.portal.Session").await?;
            let closed = proxy.receive_signal("Closed").await?.map(|_| ()).boxed();
            let options = SelectDevicesOptions::default().set_devices(Some(DeviceType::Keyboard.into()));
            timeout("selecting keyboard", portal.select_devices(&session, options)).await?.response()?;
            let response = portal.start(&session, None, Default::default()).await?.response()?;
            if !response.devices().contains(DeviceType::Keyboard) {
                bail!("RemoteDesktop session did not grant a keyboard");
            }
            Ok(closed)
        }.await;
        match setup {
            Ok(closed) => Ok(Self { session, closed, held: Vec::new(), pending: None }),
            Err(error) => {
                close(&session).await;
                Err(error)
            }
        }
    }

    pub(super) fn owns(&self, code: u32) -> bool {
        self.held.iter().any(|(source, _)| *source == code)
    }

    pub(super) fn take_closed(&mut self) -> BoxStream<'static, ()> {
        std::mem::replace(&mut self.closed, futures::stream::pending().boxed())
    }

    async fn notify(&self, portal: &RemoteDesktop, symbol: i32, down: bool) -> Result<()> {
        let state = if down { KeyState::Pressed } else { KeyState::Released };
        timeout("injecting keyboard symbol", portal.notify_keyboard_keysym(&self.session, symbol, state, Default::default())).await
    }

    pub(super) async fn send(&mut self, portal: &RemoteDesktop, message: Inject) -> Result<()> {
        match message {
            Inject::Text { text } => {
                let symbols: Vec<_> = text.as_str().chars().map(keysym).collect();
                if symbols.iter().any(|symbol| self.held.iter().any(|(_, held)| held == symbol)) { return Ok(()); }
                for symbol in symbols {
                    self.pending = Some(symbol);
                    self.notify(portal, symbol, true).await?;
                    self.notify(portal, symbol, false).await?;
                    self.pending = None;
                }
            }
            Inject::CharacterKey { code, down, character } => {
                if !character_key_code(code) || !printable(character) { return Ok(()); }
                let symbol = keysym(character);
                if let Some((_, held)) = self.held.iter().find(|(source, _)| *source == code) {
                    if *held != symbol { return Ok(()); }
                    if down { self.notify(portal, symbol, true).await?; }
                    else { self.release(portal, code).await?; }
                } else if down {
                    let already = self.held.iter().any(|(_, held)| *held == symbol);
                    self.held.push((code, symbol));
                    if !already { self.notify(portal, symbol, true).await?; }
                }
            }
            Inject::Key { code, down: false } => self.release(portal, code).await?,
            _ => {}
        }
        Ok(())
    }

    async fn release(&mut self, portal: &RemoteDesktop, code: u32) -> Result<()> {
        if let Some(index) = self.held.iter().position(|(source, _)| *source == code) {
            let symbol = self.held[index].1;
            if self.held.iter().filter(|(_, held)| *held == symbol).count() == 1 {
                self.notify(portal, symbol, false).await?;
            }
            self.held.remove(index);
        }
        Ok(())
    }

    pub(super) async fn let_go(&mut self, portal: &RemoteDesktop) -> Result<()> {
        let mut symbols = Vec::new();
        if let Some(symbol) = self.pending { symbols.push(symbol); }
        for (_, symbol) in self.held.iter().rev() {
            if !symbols.contains(symbol) { symbols.push(*symbol); }
        }
        for symbol in symbols {
            self.notify(portal, symbol, false).await?;
        }
        self.pending = None;
        self.held.clear();
        Ok(())
    }

    pub(super) async fn stop(mut self, portal: &RemoteDesktop) {
        let _ = timeout("releasing keyboard symbols", self.let_go(portal)).await;
        close(&self.session).await;
    }
}

fn keysym(character: char) -> i32 {
    let scalar = u32::from(character);
    if scalar <= 0xff { scalar as i32 } else { (0x0100_0000 | scalar) as i32 }
}

fn has_keysym_method(xml: &str) -> bool {
    let Ok(node) = zbus_xml::Node::from_reader(xml.as_bytes()) else { return false };
    node.interfaces().iter().any(|interface| {
        interface.name().as_str() == "org.freedesktop.portal.RemoteDesktop"
            && interface.methods().iter().any(|method| {
                method.name().as_str() == "NotifyKeyboardKeysym"
                    && method.args().iter().filter(|argument| argument.direction() != Some(zbus_xml::ArgDirection::Out))
                        .map(|argument| argument.ty().to_string()).collect::<Vec<_>>() == ["o", "a{sv}", "i", "u"]
                    && !method.args().iter().any(|argument| argument.direction() == Some(zbus_xml::ArgDirection::Out))
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_and_latin1_keysyms() {
        assert_eq!(keysym('z'), 0x7a);
        assert_eq!(keysym('é'), 0xe9);
        assert_eq!(keysym('ı'), 0x0100_0131);
        assert_eq!(keysym('ł'), 0x0100_0142);
    }

    #[test]
    fn require_the_right_interface_and_signature() {
        let method = "<method name='NotifyKeyboardKeysym'><arg type='o' direction='in'/><arg type='a{sv}' direction='in'/><arg type='i' direction='in'/><arg type='u' direction='in'/></method>";
        let xml = format!("<node><interface name='org.freedesktop.portal.RemoteDesktop'>{method}</interface></node>");
        assert!(has_keysym_method(&xml));
        assert!(!has_keysym_method(&xml.replace("RemoteDesktop", "InputCapture")));
        assert!(!has_keysym_method(&xml.replace("type='i'", "type='s'")));
        assert!(!has_keysym_method(&xml.replace("direction='in'", "direction='out'")));
        assert!(!has_keysym_method("not XML"));
    }
}
