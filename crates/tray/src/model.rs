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
    Stop,
    SetSide { peer: String, side: Side },
    /// The mouse and keyboard to a peer (by id), or `here`.
    Switch(String),
    LockPointer(bool),
    /// Start (true) or stop broadcast to every connected computer.
    Broadcast(bool),
    LockAll,
    /// This clipboard to a peer, by id.
    ClipSend(String),
    /// A peer's clipboard to this computer, by id.
    ClipGet(String),
    /// Pause (true) or play the live wallpaper.
    WallpaperPause(bool),
    Forget { peer: String },
    ToggleAutostart,
    /// The maker's site (Bigfoot Technology), in the browser.
    OpenMaker,
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
            Some(s) if s.controlling.is_some() || s.controlled_by.is_some() || !s.broadcast.is_empty() => IconState::Controlling,
            Some(s) if s.peers.iter().any(|p| p.online) => IconState::Connected,
            Some(_) => IconState::Idle,
        }
    }

    /// The first line of the menu, and the icon's tooltip.
    pub fn headline(&self) -> String {
        let Some(s) = &self.status else { return "oneMK isn't running".into() };
        match s.broadcast.len() {
            0 => {}
            1 => return format!("Broadcasting to {}", s.broadcast[0]),
            n => return format!("Broadcasting to {n} computers"),
        }
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
            [] => "Waiting for your computers".into(),
            _ => format!("Connected to {} computers", online.len()),
        }
    }
}

/// "Remove desk…": a menu inside the menu that asks first, because removing
/// a computer unpairs it (it has to be paired again to come back) and a
/// menu has no dialog to ask in. Choosing the first line does it; closing
/// the menu, or choosing the line above it, doesn't.
fn remove_entry(name: &str, id: &str) -> Entry {
    Entry::Sub {
        label: format!("Remove {name}…"),
        items: vec![
            Entry::Label(format!("Unpairs {name}. Pairing it again brings it back.")),
            Entry::Item { label: format!("Yes, remove {name}"), action: Action::Forget { peer: id.to_owned() }, checked: None },
        ],
    }
}

pub fn side_label(side: Side) -> &'static str {
    match side {
        Side::Left => "Left",
        Side::Right => "Right",
        Side::Top => "Above",
        Side::Bottom => "Below",
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
    m.push(Entry::Item { label: "Open oneMK…".into(), action: Action::OpenApp, checked: None });
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
                let get = online
                    .iter()
                    .map(|p| Entry::Item { label: format!("Get clipboard from {}", p.name), action: Action::ClipGet(p.id.clone()), checked: None })
                    .collect();
                grouped(&mut m, "Get clipboard from", get);
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
                        .chain([Entry::Separator, remove_entry(&p.name, &p.id)])
                        .collect(),
                });
            }
            if s.peers.is_empty() {
                m.push(Entry::Item { label: "Pair a computer…".into(), action: Action::OpenApp, checked: None });
            }
            m.push(Entry::Separator);
            let locked = s.pointer_locked;
            m.push(Entry::Item { label: "Keep the pointer here".into(), action: Action::LockPointer(!locked), checked: Some(locked) });
            // Only with somewhere to broadcast to (or to stop it).
            let broadcasting = !s.broadcast.is_empty();
            if broadcasting || s.peers.iter().any(|p| p.online) {
                m.push(Entry::Item { label: "Type on every computer".into(), action: Action::Broadcast(!broadcasting), checked: Some(broadcasting) });
            }
            if let Some(plan) = s.wallpaper.as_ref().and_then(|w| w.plan.as_ref()) {
                m.push(Entry::Item { label: "Pause the wallpaper".into(), action: Action::WallpaperPause(!plan.paused), checked: Some(plan.paused) });
            }
            m.push(Entry::Item { label: "Lock every screen".into(), action: Action::LockAll, checked: None });
            m.push(Entry::Item { label: "Take back control".into(), action: Action::TakeBack, checked: None });
            m.push(Entry::Item { label: "Stop oneMK".into(), action: Action::Stop, checked: None });
        }
        None => m.push(Entry::Item { label: "Start oneMK".into(), action: Action::StartDaemon, checked: None }),
    }
    m.push(Entry::Separator);
    m.push(Entry::Item { label: "Start at login".into(), action: Action::ToggleAutostart, checked: Some(snap.autostart) });
    m.push(Entry::Separator);
    m.push(Entry::Item { label: format!("Made by {}", one_mk_ctl::brand::MAKER), action: Action::OpenMaker, checked: None });
    m.push(Entry::Item { label: "Quit oneMK".into(), action: Action::Quit, checked: None });
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
    fn the_maker_is_a_link_just_above_quit() {
        for s in [Snapshot::default(), snap(vec![peer("a", Side::Left, true)])] {
            let m = menu(&s);
            let n = m.len();
            assert_eq!(m[n - 2], Entry::Item { label: "Made by Bigfoot Technology".into(), action: Action::OpenMaker, checked: None });
            assert!(matches!(&m[n - 1], Entry::Item { action: Action::Quit, .. }));
            assert_eq!(m[n - 3], Entry::Separator);
        }
    }

    #[test]
    fn headlines() {
        assert_eq!(Snapshot::default().headline(), "oneMK isn't running");
        assert_eq!(snap(vec![]).headline(), "No computers paired yet");
        assert_eq!(snap(vec![peer("a", Side::Left, false)]).headline(), "Waiting for your computers");
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
        assert!(menu(&s).contains(&Entry::Item { label: "Keep the pointer here".into(), action: Action::LockPointer(true), checked: Some(false) }));
        s.status.as_mut().unwrap().pointer_locked = true;
        assert!(menu(&s).contains(&Entry::Item { label: "Keep the pointer here".into(), action: Action::LockPointer(false), checked: Some(true) }));
    }

    /// Broadcast: offered only with a computer online, a checkmark while
    /// on (which stops it), and said in the headline and the icon.
    #[test]
    fn broadcast_is_a_checkmark_said_in_the_headline() {
        let item = |on: bool| Entry::Item { label: "Type on every computer".into(), action: Action::Broadcast(!on), checked: Some(on) };
        assert!(!menu(&snap(vec![peer("old", Side::Left, false)])).contains(&item(false)), "nobody to broadcast to");
        assert!(!menu(&snap(vec![])).contains(&item(false)));
        let mut s = snap(vec![peer("desk", Side::Right, true), peer("mac", Side::Left, true), peer("old", Side::Top, false)]);
        assert!(menu(&s).contains(&item(false)));
        assert_eq!(s.icon_state(), IconState::Connected);
        s.status.as_mut().unwrap().broadcast = vec!["desk".into(), "mac".into()];
        assert!(menu(&s).contains(&item(true)));
        assert_eq!(s.headline(), "Broadcasting to 2 computers");
        assert_eq!(s.icon_state(), IconState::Controlling);
        s.status.as_mut().unwrap().broadcast = vec!["a computer with a very long name".into()];
        assert_eq!(s.headline(), "Broadcasting to a computer with a very long name");
        // Still stoppable if every computer went offline meanwhile.
        let mut gone = snap(vec![peer("desk", Side::Right, false)]);
        gone.status.as_mut().unwrap().broadcast = vec!["desk".into()];
        assert!(menu(&gone).contains(&item(true)));
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
    fn nothing_in_the_menu_unpairs_in_one_click() {
        fn walk(entries: &[Entry], depth: usize, found: &mut Vec<usize>) {
            for e in entries {
                match e {
                    Entry::Item { action: Action::Forget { .. }, .. } => found.push(depth),
                    Entry::Sub { items, .. } => walk(items, depth + 1, found),
                    _ => {}
                }
            }
        }
        let mut found = vec![];
        walk(&menu(&snap(vec![peer("desk", Side::Right, true), peer("old", Side::Left, false)])), 0, &mut found);
        assert_eq!(found, vec![2, 2], "each Remove sits two menus down (computer, then the question)");
    }

    #[test]
    fn the_menu_uses_the_product_name() {
        let every = labels(&menu(&snap(vec![peer("desk", Side::Right, true)]))).join("\n") + &labels(&menu(&Snapshot::default())).join("\n");
        assert!(!every.contains("one-mk"), "{every}");
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
        // Removing unpairs: it asks first, one level down, never in one click.
        assert!(!items.iter().any(|e| matches!(e, Entry::Item { action: Action::Forget { .. }, .. })), "{items:?}");
        let Some(Entry::Sub { label, items: ask }) = items.last() else { panic!("{items:?}") };
        assert_eq!(label, "Remove desk…");
        assert!(matches!(ask.as_slice(), [Entry::Label(_), Entry::Item { label, action: Action::Forget { peer }, .. }] if peer == "desk-id" && label == "Yes, remove desk"), "{ask:?}");
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

    // ---------- QA: states and names the first tests left out ----------

    fn plan(paused: bool) -> one_mk_wallpaper::Plan {
        one_mk_wallpaper::Plan {
            hash: "a".repeat(64),
            title: "Across the horizon.mp4".into(),
            media: one_mk_wallpaper::Media { width: 3840, height: 1080, duration_ms: 10_000, frame_rate: 30 },
            canvas: Default::default(),
            start_ms: 1,
            fit: Default::default(),
            computers: vec![],
            pause_on_battery: true,
            paused,
            picture: false,
            asset_hash: None,
            spread: Default::default(),
        }
    }

    fn with_wallpaper(paused: Option<bool>) -> Snapshot {
        let mut s = snap(vec![peer("desk", Side::Right, true)]);
        let wall = one_mk_wallpaper::Status { plan: paused.map(plan), available: true, ..Default::default() };
        s.status.as_mut().unwrap().wallpaper = Some(wall);
        s
    }

    #[test]
    fn the_wallpaper_can_be_paused_and_played_from_the_menu_only_while_one_plays() {
        let item = |paused: bool| Entry::Item { label: "Pause the wallpaper".into(), action: Action::WallpaperPause(!paused), checked: Some(paused) };
        assert!(menu(&with_wallpaper(Some(false))).contains(&item(false)), "playing: choosing it pauses");
        assert!(menu(&with_wallpaper(Some(true))).contains(&item(true)), "paused: it is checked, choosing it plays");
        assert!(!labels(&menu(&with_wallpaper(None))).iter().any(|l| l.contains("wallpaper")), "nothing chosen: no entry");
        assert!(!labels(&menu(&snap(vec![]))).iter().any(|l| l.contains("wallpaper")));
    }

    #[test]
    fn with_nobody_paired_the_menu_offers_pairing_and_what_works_alone() {
        let m = menu(&snap(vec![]));
        let l = labels(&m);
        assert!(m.contains(&Entry::Item { label: "Pair a computer…".into(), action: Action::OpenApp, checked: None }), "{l:?}");
        assert!(!l.iter().any(|x| x.starts_with("Switch") || x.contains("clipboard") && x.contains("computer") || x == "Type on every computer"), "{l:?}");
        // Still there: keep the pointer, lock, take back, stop, start at login, quit.
        for want in ["Keep the pointer here", "Lock every screen", "Take back control", "Stop oneMK", "Start at login", "Quit oneMK"] {
            assert!(l.iter().any(|x| x == want), "{want}: {l:?}");
        }
        // And a paired computer takes the pairing line away.
        assert!(!labels(&menu(&snap(vec![peer("a", Side::Left, false)]))).iter().any(|x| x == "Pair a computer…"));
    }

    #[test]
    fn names_in_any_script_go_into_the_labels_whole() {
        for name in ["日本語のコンピュータ名前", "مكتب العمل الرئيسي", "שלום עולם", "Мастерская", "a very long name that goes on and on and on and on and on and on and on", "🖥️ desk", "with \"quotes\" & <angle> brackets"] {
            let s = snap(vec![peer(name, Side::Right, true)]);
            let l = labels(&menu(&s));
            for want in [format!("Switch to {name}"), format!("Send clipboard to {name}"), format!("Get clipboard from {name}"), format!("{name}: on your right"), format!("Remove {name}…"), format!("Yes, remove {name}")] {
                assert!(l.contains(&want), "{want:?} in {l:?}");
            }
            assert_eq!(s.headline(), format!("Connected to {name} (on your right)"));
        }
    }

    #[test]
    fn every_actions_peer_is_the_id_not_the_name() {
        // Two computers may share a name: the menu must say which by id.
        let mut a = peer("desk", Side::Right, true);
        let mut b = peer("desk", Side::Left, true);
        a.id = "id-a".into();
        b.id = "id-b".into();
        let m = menu(&snap(vec![a, b]));
        for id in ["id-a", "id-b"] {
            assert!(m.iter().any(|e| matches!(e, Entry::Item { action: Action::Switch(p), .. } if p == id)), "{id}: {m:?}");
            assert!(m.iter().any(|e| matches!(e, Entry::Item { action: Action::ClipSend(p), .. } if p == id)), "{id}");
            assert!(m.iter().any(|e| matches!(e, Entry::Sub { items, .. } if items.iter().any(|i| matches!(i, Entry::Item { action: Action::SetSide { peer, .. }, .. } if peer == id)))), "{id}");
        }
    }

    #[test]
    fn being_controlled_offers_the_way_back_and_names_who() {
        let mut s = snap(vec![peer("desk", Side::Right, true), peer("mac", Side::Left, true)]);
        s.status.as_mut().unwrap().controlled_by = Some("mac".into());
        assert_eq!(s.headline(), "mac is controlling this computer");
        assert_eq!(s.icon_state(), IconState::Controlling);
        let l = labels(&menu(&s));
        assert!(l.contains(&"Switch back here".into()), "{l:?}");
        assert!(l.contains(&"Take back control".into()));
    }

    #[test]
    fn the_headline_says_broadcast_before_anything_else() {
        let mut s = snap(vec![peer("a", Side::Right, true)]);
        let st = s.status.as_mut().unwrap();
        st.controlling = Some("a".into());
        st.broadcast = vec!["a".into()];
        assert_eq!(s.headline(), "Broadcasting to a");
    }

    #[test]
    fn an_unknown_transfer_size_shows_dots_and_a_stalled_one_its_percent() {
        let t = TransferStatus { id: 3, sending: true, peer: "desk".into(), files: 0, done: 10, total: 0, rate: 0 };
        assert_eq!(transfer_line(&t), "Sending files to desk…");
        let t = TransferStatus { total: 200, done: 50, rate: 0, files: 2, ..t };
        assert_eq!(transfer_line(&t), "Sending 2 files to desk: 25%");
        let t = TransferStatus { done: 900, total: 200, ..t };
        assert_eq!(transfer_line(&t), "Sending 2 files to desk: 100%", "never past 100");
    }

    #[test]
    fn side_labels_cover_every_side_and_the_menu_lists_each_once() {
        for side in Side::ALL {
            assert!(!side_label(side).is_empty());
        }
        let m = menu(&snap(vec![peer("desk", Side::Top, true)]));
        let Some(Entry::Sub { label, items }) = m.iter().find(|e| matches!(e, Entry::Sub { .. })) else { panic!("{m:?}") };
        assert_eq!(label, "desk: above");
        let sides: Vec<_> = items.iter().filter_map(|e| match e { Entry::Item { label, checked, .. } => Some((label.as_str(), *checked)), _ => None }).collect();
        assert_eq!(sides, [("Left", Some(false)), ("Right", Some(false)), ("Above", Some(true)), ("Below", Some(false))]);
    }

    #[test]
    fn start_at_login_shows_what_is_set() {
        for on in [false, true] {
            let mut s = Snapshot::default();
            s.autostart = on;
            assert!(menu(&s).contains(&Entry::Item { label: "Start at login".into(), action: Action::ToggleAutostart, checked: Some(on) }));
        }
    }
}
