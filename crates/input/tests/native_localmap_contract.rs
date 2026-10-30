const WINDOWS: &str = include_str!("../src/windows.rs");
const MACOS: &str = include_str!("../src/macos.rs");

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
    assert_eq!(canonical(function(WINDOWS, "activate")), canonical("fn activate(&mut self, side: Side, x: i32, y: i32) -> (bool, Effect) {
        self.local_fence(Destination::Remote);
        self.activate_now(side, x, y)
    }"));
    assert_eq!(canonical(function(MACOS, "start")), canonical("fn start(&self, st: &mut State, side: Side, pos: Pos) -> bool {
        self.local_fence(st, Destination::Remote);
        self.start_now(st, side, pos)
    }"));
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
