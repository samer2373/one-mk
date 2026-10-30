const WINDOWS: &str = include_str!("../src/windows_experimental.rs");
const MACOS: &str = include_str!("../src/macos_experimental.rs");

fn function<'a>(source: &'a str, name: &str) -> &'a str {
    let start = source.find(&format!("    fn {name}(" )).unwrap();
    let end = source[start..].find("\n    }").unwrap() + start + 6;
    &source[start..end]
}

fn canonical(source: &str) -> String {
    source.lines().filter(|line| !line.trim_start().starts_with("//"))
        .flat_map(str::split_whitespace).collect::<String>()
}

#[test]
fn capture_activation_is_main_plus_device_metadata_not_worker_admission() {
    let windows = function(WINDOWS, "activate_now").replace("activate_now(", "activate(")
        .replace("unsafe { PostMessageW(self.hwnd, WM_APP_DEVICES, 0, 0) };", "");
    assert_eq!(canonical(&windows), canonical(include_str!("fixtures/windows-main-capture.txt")));
    let macos = function(MACOS, "start_now").replace("start_now(", "start(")
        .replace("self.hid.capture(true);", "");
    assert_eq!(canonical(&macos), canonical(include_str!("fixtures/macos-main-capture.txt")));
    let windows = function(include_str!("../src/windows.rs"), "activate")
        .replace("unsafe { PostMessageW(self.hwnd, WM_APP_DEVICES, 0, 0) };", "");
    assert_eq!(canonical(&windows), canonical(include_str!("fixtures/windows-main-capture.txt")));
    let macos = function(include_str!("../src/macos.rs"), "start").replace("self.hid.capture(true);", "");
    assert_eq!(canonical(&macos), canonical(include_str!("fixtures/macos-main-capture.txt")));
}

#[test]
fn default_physical_callbacks_match_main_except_device_attribution() {
    let windows = include_str!("../src/windows.rs");
    let physical = ["on_mouse", "on_key", "local_input", "guard_tick"].map(|name| function(windows, name)).join("\n\n");
    assert_eq!(canonical(&physical.replace(", device: None", "")), canonical(include_str!("fixtures/windows-main-physical.txt")));
    let macos = include_str!("../src/macos.rs");
    let physical = ["on_event", "on_idle", "on_active"].map(|name| function(macos, name)).join("\n\n")
        .replace(", device: None", "")
        .replace("let send = |event: Event| self.hid.send(event);", "let send = |event: Event| { let _ = self.tx.send(event); };");
    assert_eq!(canonical(&physical), canonical(include_str!("fixtures/macos-main-physical.txt")));
}

#[test]
fn release_backend_sources_have_no_local_adapter_and_gate_is_default_off() {
    for source in [include_str!("../src/windows.rs"), include_str!("../src/macos.rs"), include_str!("../src/macos_devices.rs")] {
        for forbidden in ["localmap", "LocalRules", "local_fence", "local_rules", "receipt("] {
            assert!(!source.contains(forbidden), "default backend contains {forbidden}");
        }
    }
    assert!(include_str!("../Cargo.toml").contains("default = []"));
    assert!(include_str!("../../one-mk/Cargo.toml").contains("default = []"));
    for workflow in [include_str!("../../../.github/workflows/ci.yml"), include_str!("../../../.github/workflows/release.yml")] {
        assert!(!workflow.contains("native-local-remap"));
        assert!(!workflow.contains("--workspace --all-features"));
    }
    let modules = include_str!("../src/lib.rs");
    assert!(modules.contains("#[cfg_attr(feature = \"native-local-remap\", path = \"windows_experimental.rs\")]"));
    assert!(modules.contains("#[cfg_attr(feature = \"native-local-remap\", path = \"macos_experimental.rs\")]"));
}

#[test]
fn disabling_drops_interception_and_keeps_core_callbacks_fail_open() {
    let windows = canonical(WINDOWS);
    let macos = canonical(MACOS);
    assert!(windows.contains("ifletSome(local)=self.local_rules.take(){self.local_recovery=Some(local.retire());}"));
    let disable = WINDOWS.split("CaptureCmd::LocalRules { rules, inactive } => {").nth(1).unwrap()
        .split("if self.local_recovery.as_ref()").next().unwrap();
    assert!(disable.contains("KillTimer(self.hwnd, TIMER_LOCAL)"));
    assert!(windows.contains("matches!(wparam,4|7|18)"));
    assert!(windows.contains("ifstate.local_rules.is_none(){returnEffect::None;}"));
    assert!(canonical(function(WINDOWS, "release")).contains("local.gate.close();"));
    assert!(macos.contains("drop(st.local_rules.take());"));
    assert!(macos.contains("self.hid.local(None);"));
    assert!(macos.contains("ifheld{self.end(&mutst);}"));
    assert!(macos.contains("ifinjected&&st.local_rules.is_some(){returntrue;}"));
    let quit = MACOS.split("Msg::Quit => {").nth(1).unwrap().split("fn on_event").next().unwrap();
    assert!(quit.contains("rl.stop();"));
    assert!(!quit.contains("self.local_fence("));
}

#[test]
fn worker_recovery_and_idle_paths_remain_event_driven() {
    let windows = include_str!("../src/windows_localmap.rs");
    let macos = include_str!("../src/macos_localmap.rs");
    assert!(windows.contains("CREATE_WAITABLE_TIMER_HIGH_RESOLUTION"));
    assert!(windows.contains("CancelWaitableTimer(timer)"));
    assert!(windows.contains("Some(Work::Retire)"));
    assert!(windows.contains("!self.physical.correlated(code, down, device)"));
    assert!(windows.contains("receiver.recv().ok()"));
    assert!(macos.contains("else { std::thread::park(); }"));
    assert!(macos.contains("watchdog_feedback.healthy() && (watchdog_receipts.held"));
    assert!(include_str!("../../../Cargo.toml").contains("panic = \"abort\""));
    assert!(!windows.contains("catch_unwind"));
    assert!(!macos.contains("catch_unwind"));
    for source in [windows, macos] {
        for forbidden in [".unwrap()", ".expect(", "panic!(", "unreachable!(", "assert!("] {
            assert!(!source.contains(forbidden), "worker contains {forbidden}");
        }
    }
}

#[test]
fn swallowed_physical_takeover_precedes_local_admission() {
    let callback = MACOS.split("fn on_event(").nth(1).unwrap().split("fn on_active(").next().unwrap();
    assert!(callback.find("st.local_gate.check").unwrap() < callback.find("local.hook(").unwrap());
    assert!(callback.contains("if !st.active && CGEvent::integer_value_field(e, CGEventField::EventSourceStateID)"));
}

#[test]
fn local_dispatch_requires_its_own_sequence_not_a_matching_remote_key() {
    for source in [include_str!("../src/windows_localmap.rs"), include_str!("../src/macos_localmap.rs")] {
        let forward = source.split("pub fn forward(").nth(1).unwrap().split("\n    }").next().unwrap();
        assert!(forward.contains("let Some(sequence) = sequence else { return false }"));
        assert!(forward.contains("else { self.fail(); return true }"));
    }
    assert!(include_str!("../src/macos_devices_experimental.rs").contains("if event.local.is_some() { return; }"));
    assert!(WINDOWS.contains("if ev.local.is_some() { continue; }"));
}

#[test]
fn activation_waits_for_serialized_release_completion() {
    assert!(function(WINDOWS, "activate").contains("local.activation = Some"));
    assert!(function(MACOS, "start").contains("local.activation = Some"));
    for source in [WINDOWS, MACOS] {
        let pump = function(source, "local_pump");
        assert!(pump.find("if !done.success").unwrap() < pump.find("activation.take()").unwrap());
    }
}
