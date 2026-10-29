//! What the menu says and does, independent of the platform that draws it.

use one_mk_ctl::status::{Status, TransferStatus, on_your};
use crate::icon::IconState;
use one_mk_proto::Side;

/// What the menu is built from: a fresh look at the daemon and the login settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// `None` when the daemon is not running (or does not answer).
    pub status: Option<Status>,
    pub autostart: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    OpenApp,
    StartDaemon,
    TakeBack,
    SetSide { peer: String, side: Side },
    /// The mouse and keyboard to a peer (by id), or `here`.
    Switch(String),
    LockPointer(bool),
    LockAll,
    /// This clipboard to a peer, by id.
    ClipSend(String),
    ToggleAutostart,
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// Text that does nothing.
    Label(String),
    Separator,
    Item { label: String, action: Action, checked: Option<bool> },
    Sub { label: String, items: Vec<Entry> },
}

impl Snapshot {
    #[cfg(any(windows, target_os = "macos"))]
    pub fn running(&self) -> bool {
        self.status.is_some()
    }

    /// Which icon to draw.
    pub fn icon_state(&self) -> IconState {
        match &self.status {
            None => IconState::Off,
            Some(s) if s.controlling.is_some() || s.controlled_by.is_some() => IconState::Controlling,
            Some(s) if s.peers.iter().any(|p| p.online) => IconState::Connected,
            Some(_) => IconState::Idle,
        }
    }

    /// The first line of the menu, and the icon's tooltip.
    pub fn headline(&self) -> String {
        let Some(s) = &self.status else { return "one-mk is not running".into() };
        if let Some(p) = &s.controlled_by {
            return format!("{p} is controlling this computer");
        }
        if let Some(p) = &s.controlling {
            return format!("Controlling {p}");
        }
        let online: Vec<_> = s.peers.iter().filter(|p| p.online).collect();
        match online[..] {
            [p] => format!("Connected to {} ({})", p.name, on_your(p.side)),
            [] if s.peers.is_empty() => "No computers paired yet".into(),
            [] => "Not connected to any computer".into(),
            _ => format!("Connected to {} computers", online.len()),
        }
    }
}

pub fn side_label(side: Side) -> &'static str {
    match side {
        Side::Left => "Left",
        Side::Right => "Right",
        Side::Top => "Top",
        Side::Bottom => "Bottom",
    }
}

/// "Receiving 3 files from desk: 45%", for one line of the menu.
pub fn transfer_line(t: &TransferStatus) -> String {
    let what = match t.files {
        0 => "files".to_owned(),
        1 => "1 file".to_owned(),
        n => format!("{n} files"),
    };
    let head = if t.sending { format!("Sending {what} to {}", t.peer) } else { format!("Receiving {what} from {}", t.peer) };
    match t.percent() {
        Some(p) => format!("{head}: {p}%"),
        None => format!("{head}…"),
    }
}

/// Up to this many computers get their own entries; more go in a submenu.
const FLAT: usize = 3;

/// Entries, or one submenu of them when there are many.
fn grouped(m: &mut Vec<Entry>, title: &str, items: Vec<Entry>) {
    if items.len() > FLAT {
        m.push(Entry::Sub { label: title.into(), items });
    } else {
        m.extend(items);
    }
}

pub fn menu(snap: &Snapshot) -> Vec<Entry> {
    let mut m = vec![Entry::Label(snap.headline())];
    if let Some(s) = &snap.status {
        m.extend(s.transfers.iter().map(|t| Entry::Label(transfer_line(t))));
    }
    m.push(Entry::Separator);
    m.push(Entry::Item { label: "Open one·mk…".into(), action: Action::OpenApp, checked: None });
    m.push(Entry::Separator);
    match &snap.status {
        Some(s) => {
            let online: Vec<_> = s.peers.iter().filter(|p| p.online).collect();
            if !online.is_empty() {
                let away = s.controlling.is_some() || s.controlled_by.is_some();
                let mut switch: Vec<Entry> = online
                    .iter()
                    .filter(|p| s.controlling.as_ref() != Some(&p.name))
                    .map(|p| Entry::Item { label: format!("Switch to {}", p.name), action: Action::Switch(p.id.clone()), checked: None })
                    .collect();
                if away {
                    switch.insert(0, Entry::Item { label: "Switch back here".into(), action: Action::Switch("here".into()), checked: None });
                }
                grouped(&mut m, "Switch to", switch);
                let clip = online
                    .iter()
                    .map(|p| Entry::Item { label: format!("Send clipboard to {}", p.name), action: Action::ClipSend(p.id.clone()), checked: None })
                    .collect();
                grouped(&mut m, "Send clipboard to", clip);
                m.push(Entry::Separator);
            }
            for p in &s.peers {
                let state = if p.online { "" } else { " (offline)" };
                m.push(Entry::Sub {
                    label: format!("{}: {}{state}", p.name, on_your(p.side)),
                    items: Side::ALL
                        .into_iter()
                        .map(|side| Entry::Item {
                            label: side_label(side).into(),
                            action: Action::SetSide { peer: p.id.clone(), side },
                            checked: Some(p.side == side),
                        })
                        .collect(),
                });
            }
            if s.peers.is_empty() {
                m.push(Entry::Label("Run `one-mk pair` on both computers".into()));
            }
            m.push(Entry::Separator);
            let locked = s.pointer_locked;
            m.push(Entry::Item { label: "Lock pointer here".into(), action: Action::LockPointer(!locked), checked: Some(locked) });
            m.push(Entry::Item { label: "Lock all computers".into(), action: Action::LockAll, checked: None });
            m.push(Entry::Item { label: "Take back control".into(), action: Action::TakeBack, checked: None });
        }
        None => m.push(Entry::Item { label: "Start one-mk".into(), action: Action::StartDaemon, checked: None }),
    }
    m.push(Entry::Separator);
    m.push(Entry::Item { label: "Start at login".into(), action: Action::ToggleAutostart, checked: Some(snap.autostart) });
    m.push(Entry::Item { label: "Quit tray".into(), action: Action::Quit, checked: None });
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_ctl::status::PeerStatus;

    fn peer(name: &str, side: Side, online: bool) -> PeerStatus {
        PeerStatus { id: format!("{name}-id"), name: name.into(), side, os: 0, online, swap: "auto".into() }
    }

    fn snap(peers: Vec<PeerStatus>) -> Snapshot {
        Snapshot { status: Some(Status { name: "me".into(), peers, ..Default::default() }), autostart: false }
    }

    #[test]
    fn headlines() {
        assert_eq!(Snapshot::default().headline(), "one-mk is not running");
        assert_eq!(snap(vec![]).headline(), "No computers paired yet");
        assert_eq!(snap(vec![peer("a", Side::Left, false)]).headline(), "Not connected to any computer");
        assert_eq!(snap(vec![peer("desk", Side::Right, true)]).headline(), "Connected to desk (on your right)");
        assert_eq!(snap(vec![peer("a", Side::Right, true), peer("b", Side::Top, true)]).headline(), "Connected to 2 computers");
        let mut s = snap(vec![peer("a", Side::Right, true)]);
        s.status.as_mut().unwrap().controlled_by = Some("a".into());
        assert_eq!(s.headline(), "a is controlling this computer");
    }

    #[test]
    fn icon_states() {
        assert_eq!(Snapshot::default().icon_state(), IconState::Off);
        assert_eq!(snap(vec![]).icon_state(), IconState::Idle);
        assert_eq!(snap(vec![peer("a", Side::Left, false)]).icon_state(), IconState::Idle);
        let mut s = snap(vec![peer("a", Side::Left, true)]);
        assert_eq!(s.icon_state(), IconState::Connected);
        s.status.as_mut().unwrap().controlling = Some("a".into());
        assert_eq!(s.icon_state(), IconState::Controlling);
    }

    #[test]
    fn open_app_comes_right_after_the_headline() {
        for s in [Snapshot::default(), snap(vec![peer("a", Side::Left, true)])] {
            let m = menu(&s);
            assert!(matches!(m[1], Entry::Separator));
            assert!(matches!(m[2], Entry::Item { action: Action::OpenApp, .. }), "{m:?}");
        }
    }

    fn labels(m: &[Entry]) -> Vec<String> {
        m.iter()
            .flat_map(|e| match e {
                Entry::Label(l) | Entry::Item { label: l, .. } => vec![l.clone()],
                Entry::Sub { label, items } => std::iter::once(label.clone()).chain(labels(items)).collect(),
                Entry::Separator => vec![],
            })
            .collect()
    }

    #[test]
    fn online_computers_get_switch_and_clipboard_entries() {
        let m = menu(&snap(vec![peer("desk", Side::Right, true), peer("old", Side::Left, false)]));
        let l = labels(&m);
        assert!(l.contains(&"Switch to desk".into()) && l.contains(&"Send clipboard to desk".into()), "{l:?}");
        assert!(!l.iter().any(|x| x == "Switch to old" || x == "Send clipboard to old"), "offline computers have none: {l:?}");
        assert!(!l.contains(&"Switch back here".into()));
        assert!(m.contains(&Entry::Item { label: "Switch to desk".into(), action: Action::Switch("desk-id".into()), checked: None }));
        assert!(m.iter().any(|e| matches!(e, Entry::Item { action: Action::LockAll, .. })));
        // Controlling desk: back here, and no "switch to" the one already in use.
        let mut s = snap(vec![peer("desk", Side::Right, true)]);
        s.status.as_mut().unwrap().controlling = Some("desk".into());
        let l = labels(&menu(&s));
        assert!(l.contains(&"Switch back here".into()) && !l.contains(&"Switch to desk".into()), "{l:?}");
        // Many computers: one submenu each instead of a long menu.
        let many = menu(&snap((0..5).map(|i| peer(&format!("pc{i}"), Side::Right, true)).collect()));
        assert!(many.iter().any(|e| matches!(e, Entry::Sub { label, items } if label == "Switch to" && items.len() == 5)), "{many:?}");
    }

    #[test]
    fn pointer_lock_is_a_checkmark_that_flips() {
        let mut s = snap(vec![peer("desk", Side::Right, true)]);
        assert!(menu(&s).contains(&Entry::Item { label: "Lock pointer here".into(), action: Action::LockPointer(true), checked: Some(false) }));
        s.status.as_mut().unwrap().pointer_locked = true;
        assert!(menu(&s).contains(&Entry::Item { label: "Lock pointer here".into(), action: Action::LockPointer(false), checked: Some(true) }));
    }

    #[test]
    fn a_running_transfer_shows_under_the_headline() {
        let mut s = snap(vec![peer("desk", Side::Right, true)]);
        let t = TransferStatus { id: 1, sending: false, peer: "desk".into(), files: 3, done: 45, total: 100, rate: 10 };
        s.status.as_mut().unwrap().transfers = vec![t.clone()];
        let m = menu(&s);
        assert_eq!(m[1], Entry::Label("Receiving 3 files from desk: 45%".into()));
        assert_eq!(transfer_line(&TransferStatus { sending: true, files: 1, total: 0, ..t }), "Sending 1 file to desk…");
    }

    #[test]
    fn not_running_offers_to_start() {
        let m = menu(&Snapshot::default());
        assert!(m.iter().any(|e| matches!(e, Entry::Item { action: Action::StartDaemon, .. })));
        assert!(!m.iter().any(|e| matches!(e, Entry::Item { action: Action::TakeBack, .. })));
        assert!(m.iter().any(|e| matches!(e, Entry::Item { action: Action::Quit, .. })));
    }

    #[test]
    fn each_device_gets_a_side_submenu_with_the_current_one_checked() {
        let m = menu(&snap(vec![peer("desk", Side::Right, true)]));
        let Some(Entry::Sub { label, items }) = m.iter().find(|e| matches!(e, Entry::Sub { .. })) else { panic!("{m:?}") };
        assert_eq!(label, "desk: on your right");
        assert_eq!(items.len(), 4);
        let checked: Vec<_> = items
            .iter()
            .filter_map(|e| match e {
                Entry::Item { action: Action::SetSide { side, peer }, checked: Some(true), .. } => Some((*side, peer.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(checked, vec![(Side::Right, "desk-id".to_string())]);
        assert!(m.iter().any(|e| matches!(e, Entry::Item { action: Action::TakeBack, .. })));
        assert!(!m.iter().any(|e| matches!(e, Entry::Item { action: Action::StartDaemon, .. })));
    }
}
