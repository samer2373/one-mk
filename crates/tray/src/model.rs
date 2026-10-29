//! What the menu says and does, independent of the platform that draws it.

use one_mk_ctl::status::{Status, on_your};
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
    Stop,
    SetSide { peer: String, side: Side },
    Forget { peer: String },
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

pub fn menu(snap: &Snapshot) -> Vec<Entry> {
    let mut m = vec![
        Entry::Label(snap.headline()),
        Entry::Separator,
        Entry::Item { label: "Open one·mk…".into(), action: Action::OpenApp, checked: None },
        Entry::Separator,
    ];
    match &snap.status {
        Some(s) => {
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
                        .chain([
                            Entry::Separator,
                            Entry::Item { label: format!("Remove {}", p.name), action: Action::Forget { peer: p.id.clone() }, checked: None },
                        ])
                        .collect(),
                });
            }
            if s.peers.is_empty() {
                m.push(Entry::Label("Run `one-mk pair` on both computers".into()));
            }
            m.push(Entry::Separator);
            m.push(Entry::Item { label: "Take back control".into(), action: Action::TakeBack, checked: None });
            m.push(Entry::Item { label: "Stop one-mk".into(), action: Action::Stop, checked: None });
        }
        None => m.push(Entry::Item { label: "Start one-mk".into(), action: Action::StartDaemon, checked: None }),
    }
    m.push(Entry::Separator);
    m.push(Entry::Item { label: "Start at login".into(), action: Action::ToggleAutostart, checked: Some(snap.autostart) });
    m.push(Entry::Item { label: "Quit one·mk".into(), action: Action::Quit, checked: None });
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_ctl::status::PeerStatus;

    fn peer(name: &str, side: Side, online: bool) -> PeerStatus {
        PeerStatus { id: format!("{name}-id"), name: name.into(), side, os: 0, online, swap: "auto".into(), screens: vec![] }
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
        assert_eq!(items.len(), 6, "four sides, a line, and Remove");
        assert!(matches!(items.last(), Some(Entry::Item { action: Action::Forget { peer }, .. }) if peer == "desk-id"));
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
