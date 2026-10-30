use super::*;
use one_mk_input::typing::KeyboardLayout;

#[cfg(target_os = "linux")]
#[path = "typing_process.rs"]
mod processes;

#[tokio::test]
async fn physical_emergency_chord_precedes_mapping_in_character_visit() {
    let mut duo = Duo::start_typing("typing-emergency", "de", "us").await;
    Settings::update(&duo.da, |settings| {
        settings.input.keymap.rule = vec![crate::tuning::KeyRule { from: "esc".into(), to: Some(crate::tuning::To::One("y".into())), ..Default::default() }];
        Ok(())
    }).unwrap();
    control::request(&duo.da, Request::Reload).await.unwrap();
    duo.enter_b().await;
    for code in [29, 56, 42] {
        duo.ha.key_from(code, true, None);
        assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code, down: true });
    }
    duo.ha.key_from(1, true, None);
    assert!(matches!(next_capture(&mut duo.ha).await, CaptureCmd::Release { .. }));
    let mut releases = HashSet::new();
    for _ in 0..3 {
        let Inject::Key { code, down: false } = next_non_move(&mut duo.hb).await else { panic!("emergency chord was mapped or typed") };
        releases.insert(code);
    }
    assert_eq!(releases, HashSet::from([29, 56, 42]));
    duo.stop().await;
}

#[tokio::test]
async fn enabled_backend_still_requires_peer_text_negotiation() {
    let mut duo = Duo::start_typing("typing-caps-required", "de", "us").await;
    let mut peer = RawPeer::connect("typing-legacy", &duo.db, duo.pb).await;
    peer.send(Control::Enter { epoch: 13, pos: Pos::new(1.0, 1.0) }).await;
    assert_eq!(peer.next().await, Control::EnterAck { epoch: 13 });
    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(1.0, 1.0)));
    peer.send(Control::Text { text: "@".into() }).await;
    peer.send(Control::CharacterKey { code: 21, down: true, character: 'z' }).await;
    peer.send(Control::Key { code: 30, down: true }).await;
    assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 30, down: true });
    drop(peer);
    duo.stop().await;
}

#[tokio::test]
async fn missing_text_capability_and_equal_layouts_keep_physical_keys() {
    for receiver in [None, Some(KeyboardLayout::fake("de"))] {
        let desk = Desktop::new(vec![Rect::new(0, 0, 1920, 1080)]);
        let mut duo = Duo::start_on_keyboards("typing-fallback", None, |_, _| {}, desk.clone(), desk, (Some(KeyboardLayout::fake("de")), receiver)).await;
        duo.settle().await;
        Settings::update(&duo.da, |settings| { settings.typing.keys = crate::typing::Mode::Auto; Ok(()) }).unwrap();
        control::request(&duo.da, Request::Reload).await.unwrap();
        duo.enter_b().await;
        duo.ha.key_from(21, true, None);
        duo.ha.key_from(21, false, None);
        assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 21, down: true });
        assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 21, down: false });
        duo.stop().await;
    }
}

#[tokio::test]
async fn character_resolution_uses_mapper_output_before_control_meta_swap() {
    let mut duo = Duo::start_typing("typing-mapper", "de", "us").await;
    Settings::update(&duo.da, |settings| {
        settings.input.keymap.rule = vec![crate::tuning::KeyRule { from: "a".into(), to: Some(crate::tuning::To::One("y".into())), ..Default::default() }];
        Ok(())
    }).unwrap();
    control::request(&duo.da, Request::Reload).await.unwrap();
    Duo::layout(&duo.da, &duo.fb, None, Some(Swap::On)).await;
    duo.enter_b().await;
    duo.ha.key_from(30, true, None);
    duo.ha.key_from(30, false, None);
    assert_eq!(next_inject(&mut duo.hb).await, Inject::Text { text: "z".into() });
    duo.ha.key_from(29, true, None);
    duo.ha.key_from(30, true, None);
    assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 125, down: true });
    assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 44, down: true });
    duo.stop().await;
}

#[test]
fn randomized_owner_events_never_orphan_a_release() {
    let peer = Identity::ephemeral().unwrap().fingerprint;
    let mut owner = Owner::new(peer, 1, 1, Pos::new(0.0, 0.0));
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let inject = InjectTx::plain(sender);
    let layouts = [KeyboardLayout::fake("us"), KeyboardLayout::fake("de"), KeyboardLayout::fake("fr")];
    let mut seed = 0xfeed_face_1234_5678u64;
    let mut held = HashSet::new();
    for index in 0..20_000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let code = [21, 30, 44, 29, 42, 100][(seed >> 32) as usize % 6];
        let down = seed & 2 != 0;
        if seed & 1 == 0 {
            owner.input(Control::Key { code, down }, &inject);
        } else {
            owner.character_input(Control::CharacterKey { code, down, character: if seed & 4 == 0 { 'z' } else { 'y' } }, &layouts[index % 3], &inject);
        }
        if index % 31 == 0 {
            owner.release_all(&inject);
            assert!(owner.source_kinds.is_empty());
            owner = Owner::new(peer, 1, 1, Pos::new(0.0, 0.0));
        }
        while let Ok(event) = receiver.try_recv() {
            if let Inject::Key { code, down } = event {
                if down { held.insert(code); } else { assert!(held.remove(&code), "orphan up {code} at {index}"); }
            }
        }
    }
    owner.release_all(&inject);
    while let Ok(Inject::Key { code, down: false }) = receiver.try_recv() { assert!(held.remove(&code)); }
    assert!(held.is_empty());
    assert!(owner.source_kinds.is_empty());
}

#[tokio::test]
async fn national_levels_and_layout_change_mid_hold() {
    for (name, code, shift, altgr, expected) in [
        ("de", 44, false, false, "y"),
        ("de", 16, false, true, "@"),
        ("fr", 16, false, false, "a"),
        ("fr", 11, false, true, "@"),
        ("fr", 5, false, true, "{"),
        ("pl", 38, false, true, "ł"),
        ("pl", 30, true, true, "Ą"),
        ("tr", 23, false, false, "ı"),
    ] {
        let mut duo = Duo::start_typing(&format!("national-{name}-{code}-{shift}"), name, "us").await;
        duo.enter_b().await;
        for modifier in [shift.then_some(42), altgr.then_some(100)].into_iter().flatten() {
            duo.ha.key_from(modifier, true, None);
            assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: modifier, down: true });
        }
        duo.ha.key_from(code, true, None);
        assert_eq!(next_inject(&mut duo.hb).await, Inject::Text { text: expected.into() });
        duo.ha.events.send(Event::KeyboardLayout(KeyboardLayout::fake("us"))).unwrap();
        duo.ha.key_from(code, true, None);
        duo.ha.key_from(code, false, None);
        assert_eq!(next_inject(&mut duo.hb).await, Inject::Text { text: expected.into() });
        duo.leave_b().await;
        duo.stop().await;
    }
}

#[tokio::test]
async fn receiver_remembers_resolved_code_across_layout_change_and_disconnect() {
    let mut duo = Duo::start_typing("receiver-change", "de", "us").await;
    duo.enter_b().await;
    duo.ha.key_from(29, true, None);
    duo.ha.key_from(21, true, None);
    assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 29, down: true });
    assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 44, down: true });
    duo.hb.events.send(Event::KeyboardLayout(KeyboardLayout::fake("de"))).unwrap();
    duo.ha.key_from(21, false, None);
    assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 44, down: false });
    duo.tasks[0].abort();
    assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 29, down: false });
    duo.stop.send(true).unwrap();
    for task in duo.tasks { let _ = task.await; }
    let _ = std::fs::remove_dir_all(duo.da);
    let _ = std::fs::remove_dir_all(duo.db);
}

#[tokio::test]
async fn enabled_path_refuses_non_owner_spoofed_keys_invalid_text_and_over_budget() {
    let mut duo = Duo::start_typing("hostile-enabled", "de", "us").await;
    let mut hostile = RawPeer::connect("enabled-owner", &duo.db, duo.pb).await;
    hostile.send(Control::Caps(caps::TEXT)).await;
    assert!(matches!(hostile.next().await, Control::KeyboardLayout(_)));
    hostile.send(Control::Text { text: "x".into() }).await;
    hostile.send(Control::CharacterKey { code: 21, down: true, character: 'z' }).await;
    hostile.send(Control::Enter { epoch: 91, pos: Pos::new(5.0, 5.0) }).await;
    assert_eq!(hostile.next().await, Control::EnterAck { epoch: 91 });
    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(5.0, 5.0)));
    let mut outsider = RawPeer::connect("enabled-outsider", &duo.db, duo.pb).await;
    outsider.send(Control::Caps(caps::TEXT)).await;
    assert!(matches!(outsider.next().await, Control::KeyboardLayout(_)));
    outsider.send(Control::Text { text: "x".into() }).await;
    outsider.send(Control::CharacterKey { code: 21, down: true, character: 'z' }).await;
    for code in [1, 15, 28, 29, 42, 56, 97, 100, 125, 767] {
        hostile.send(Control::CharacterKey { code, down: true, character: 'z' }).await;
    }
    for text in ["\n", "\t", "\x1b", "\u{202e}", "\u{2066}", "\u{200d}", "ab", "e\u{301}\u{301}\u{301}\u{301}"] {
        let mut body = vec![21];
        body.extend_from_slice(&(text.len() as u16).to_le_bytes());
        body.extend_from_slice(text.as_bytes());
        hostile.send_body(&body).await;
    }
    hostile.send(Control::CharacterKey { code: 21, down: true, character: 'z' }).await;
    assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 44, down: true });
    hostile.send(Control::Key { code: 21, down: false }).await;
    hostile.send(Control::CharacterKey { code: 21, down: false, character: 'y' }).await;
    for _ in 1..one_mk_proto::typing::TEXT_PER_SECOND {
        hostile.send(Control::Text { text: "@".into() }).await;
        assert_eq!(next_inject(&mut duo.hb).await, Inject::Text { text: "@".into() });
    }
    for _ in 0..10 { hostile.send(Control::Text { text: "x".into() }).await; }
    hostile.send(Control::CharacterKey { code: 21, down: false, character: 'z' }).await;
    assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 44, down: false });
    hostile.send(Control::Key { code: 30, down: true }).await;
    hostile.send(Control::CharacterKey { code: 30, down: false, character: 'z' }).await;
    hostile.send(Control::Leave).await;
    assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 30, down: true });
    assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 30, down: false });
    drop((hostile, outsider));
    duo.stop().await;
}

#[test]
fn receiver_budget_never_blocks_matching_release_or_accepts_shortcut_text() {
    let peer = Identity::ephemeral().unwrap().fingerprint;
    let mut owner = Owner::new(peer, 1, 1, Pos::new(0.0, 0.0));
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let inject = InjectTx::plain(sender);
    let layout = KeyboardLayout::fake("us");
    owner.input(Control::Key { code: 29, down: true }, &inject);
    owner.character_input(Control::Text { text: "@".into() }, &layout, &inject);
    assert_eq!(receiver.try_recv().unwrap(), Inject::Key { code: 29, down: true });
    assert!(receiver.try_recv().is_err());
    owner.character_input(Control::CharacterKey { code: 21, down: true, character: 'z' }, &layout, &inject);
    assert_eq!(receiver.try_recv().unwrap(), Inject::Key { code: 44, down: true });
    for _ in 0..one_mk_proto::typing::TEXT_PER_SECOND {
        owner.character_input(Control::CharacterKey { code: 21, down: true, character: 'z' }, &layout, &inject);
    }
    while receiver.try_recv().is_ok() {}
    owner.character_input(Control::CharacterKey { code: 21, down: false, character: 'z' }, &layout, &inject);
    assert_eq!(receiver.try_recv().unwrap(), Inject::Key { code: 44, down: false });
    owner.release_all(&inject);
    assert_eq!(receiver.try_recv().unwrap(), Inject::Key { code: 29, down: false });
    assert!(owner.source_kinds.is_empty());
}
