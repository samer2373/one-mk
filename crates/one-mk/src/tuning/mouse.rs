use super::*;

#[derive(clap::Args, Debug, Default)]
#[command(mut_arg("device", |arg| arg.hide(true)))]
pub struct AutoscrollArgs {
    /// Enable click-to-scroll on the other computer.
    #[arg(long, conflicts_with = "off")]
    pub on: bool,
    /// Disable click-to-scroll.
    #[arg(long)]
    pub off: bool,
    /// Toggle button, for example middle or button4 (default middle).
    #[arg(long)]
    pub button: Option<String>,
    /// Scroll rate multiplier, from 0.1 to 10 (default 1).
    #[arg(long, allow_negative_numbers = true)]
    pub speed: Option<f64>,
    /// Movement before scrolling starts, 0 to 1000 backend motion units (default 8).
    #[arg(long, allow_negative_numbers = true)]
    pub dead_zone: Option<f64>,
    /// Scroll only up and down, or in both directions.
    #[arg(long, value_name = "on|off", value_parser = settings::parse_on_off)]
    pub vertical_only: Option<bool>,
    /// Forget the autoscroll settings for this scope (and app), so the ones
    /// for everything else apply again.
    #[arg(long, conflicts_with_all = ["on", "off", "button", "speed", "dead_zone", "vertical_only"])]
    pub reset: bool,
    #[command(flatten)]
    pub scope: ScopeArgs,
}

/// The gestures autoscroll's toggle button shadows, as `check` words them.
fn reserved_gestures(input: &TuningSettings, paired: &[(String, String, u8)]) -> Vec<String> {
    input.check(paired).into_iter().filter(|problem| problem.contains("reserved by autoscroll")).collect()
}

/// Autoscroll and swipes belong to the pointer of the visit, not to one
/// device: a rule for one mouse would never apply, so it is refused.
fn no_device(scope: &ScopeArgs) -> Result<()> {
    if scope.device.is_some() {
        bail!("autoscroll and gestures can't be set for one keyboard or mouse (--device): they apply to the pointer of a visit. Use --peer or --os to narrow them");
    }
    Ok(())
}

/// Folds the autoscroll tables for one place (`scope` and `app`) into one,
/// later fields over earlier ones as when resolving: the base counts as
/// the first table for an unscoped place, so a table written by hand for
/// the same place can't keep overriding what an edit sets.
fn fold_autoscroll_rules(auto: &mut AutoscrollSettings, scope: &Scope, app: &Option<String>) {
    let here = |rule: &AutoscrollRule| rule.scope == *scope && same_app(&rule.app, app);
    // The table the others go into: the base, or the first one for the place.
    let into = if scope.is_global() && app.is_none() { None } else {
        match auto.rule.iter().position(here) {
            Some(first) => Some(first),
            None => return,
        }
    };
    let mut i = into.map_or(0, |first| first + 1);
    while i < auto.rule.len() {
        if !here(&auto.rule[i]) {
            i += 1;
            continue;
        }
        let later = auto.rule.remove(i);
        let target = match into {
            Some(first) => &mut auto.rule[first],
            None => &mut auto.base,
        };
        target.enabled = later.enabled.or(target.enabled);
        if later.button.is_some() {
            target.button = later.button;
        }
        target.speed = later.speed.or(target.speed);
        target.dead_zone = later.dead_zone.or(target.dead_zone);
        target.vertical_only = later.vertical_only.or(target.vertical_only);
    }
}

pub async fn autoscroll_command(dir: &Path, args: AutoscrollArgs) -> Result<()> {
    no_device(&args.scope)?;
    let scope = args.scope.scope(dir)?;
    let app = args.scope.app()?;
    if !args.reset && !args.on && !args.off && args.button.is_none() && args.speed.is_none() && args.dead_zone.is_none() && args.vertical_only.is_none() {
        let settings = Settings::view(dir)?;
        for rule in settings.input.autoscroll.rules() {
            let global = rule.scope.is_global() && rule.app.is_none();
            let toggle = |value: Option<bool>, default| value.or(global.then_some(default))
                .map_or("inherit", |enabled| if enabled { "on" } else { "off" });
            let number = |value: Option<f64>, default| value.or(global.then_some(default))
                .map_or_else(|| "inherit".to_owned(), |number| number.to_string());
            println!("Autoscroll{}: {} · button {} · speed {} · dead zone {} · vertical only {}",
                where_suffix(&rule.scope, &rule.app), toggle(rule.enabled, false),
                rule.button.as_deref().unwrap_or(if global { "middle" } else { "inherit" }),
                number(rule.speed, 1.0), number(rule.dead_zone, 8.0), toggle(rule.vertical_only, false));
        }
        return Ok(());
    }
    if args.reset {
        let removed = Settings::update(dir, |settings| {
            let auto = &mut settings.input.autoscroll;
            if scope.is_global() && app.is_none() {
                // The base, and any table for the same place written by hand.
                let before = auto.rule.len();
                auto.rule.retain(|rule| !(rule.scope.is_global() && rule.app.is_none()));
                let had = auto.base != AutoscrollRule::default() || auto.rule.len() != before;
                auto.base = AutoscrollRule::default();
                Ok(had)
            } else {
                let before = auto.rule.len();
                auto.rule.retain(|rule| !(rule.scope == scope && same_app(&rule.app, &app)));
                Ok(auto.rule.len() != before)
            }
        })?;
        if removed {
            println!("Autoscroll settings forgotten{}.", where_suffix(&scope, &app));
        } else {
            println!("There were no autoscroll settings{} to forget.", where_suffix(&scope, &app));
        }
        return settings::apply(dir).await;
    }
    let (scope_shown, app_shown) = (scope.clone(), app.clone());
    let update = AutoscrollRule { enabled: if args.on { Some(true) } else if args.off { Some(false) } else { None },
        button: args.button, speed: args.speed, dead_zone: args.dead_zone, vertical_only: args.vertical_only,
        scope: scope.clone(), app: app.clone() };
    update.validate().map_err(|error| anyhow::anyhow!("invalid autoscroll option: {error}"))?;
    let paired = paired(dir);
    Settings::update(dir, |settings| {
        let known = reserved_gestures(&settings.input, &paired);
        let auto = &mut settings.input.autoscroll;
        fold_autoscroll_rules(auto, &scope, &app);
        let rule = if scope.is_global() && app.is_none() { &mut auto.base } else {
            if !auto.rule.iter().any(|rule| rule.scope == scope && same_app(&rule.app, &app)) {
                auto.rule.push(AutoscrollRule { scope, app, ..Default::default() });
            }
            auto.rule.iter_mut().find(|rule| rule.scope == update.scope && same_app(&rule.app, &update.app))
                .ok_or_else(|| anyhow::anyhow!("could not find the autoscroll setting; try again"))?
        };
        rule.enabled = update.enabled.or(rule.enabled);
        rule.button = update.button.or_else(|| rule.button.clone());
        rule.speed = update.speed.or(rule.speed);
        rule.dead_zone = update.dead_zone.or(rule.dead_zone);
        rule.vertical_only = update.vertical_only.or(rule.vertical_only);
        if let Some(problem) = reserved_gestures(&settings.input, &paired).into_iter().find(|problem| !known.contains(problem)) {
            bail!("{problem}");
        }
        Ok(())
    })?;
    if args.off {
        println!("Autoscroll disabled{}; the toggle button no longer starts scrolling.", where_suffix(&scope_shown, &app_shown));
    } else if args.on {
        println!("Autoscroll is on{}. Click the toggle button, then move away from it to scroll; click again, press Esc, or leave the computer to stop.", where_suffix(&scope_shown, &app_shown));
    } else {
        println!("Autoscroll settings saved{}.", where_suffix(&scope_shown, &app_shown));
    }
    settings::apply(dir).await
}

#[derive(clap::Subcommand, Debug)]
pub enum GesturesCmd {
    /// Show held-button gestures.
    List,
    /// Hold BUTTON and swipe DIRECTION to type TO on the other computer.
    #[command(mut_arg("device", |arg| arg.hide(true)))]
    Add {
        /// Button to hold, for example button4 (not left or right).
        button: String,
        /// Direction to swipe while holding the button.
        #[arg(value_parser = ["up", "down", "left", "right"], ignore_case = true)]
        direction: String,
        /// A key, chord, OS shortcut (os:next-desktop), quoted action, or sequence;
        /// desktop-left and desktop-right switch virtual desktops on any OS.
        #[arg(required = true, num_args = 1..)]
        to: Vec<String>,
        /// Minimum swipe distance, 1 to 10000 backend motion units.
        #[arg(long, default_value_t = 60.0)]
        threshold: f64,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Remove a button's gesture in this scope.
    #[command(mut_arg("device", |arg| arg.hide(true)))]
    Remove {
        /// Button whose gesture should be removed.
        button: String,
        /// Direction whose gesture should be removed.
        #[arg(value_parser = ["up", "down", "left", "right"], ignore_case = true)]
        direction: String,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Remove all held-button gestures in this scope.
    #[command(mut_arg("device", |arg| arg.hide(true)))]
    Clear {
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Hold the back side button and swipe left or right between desktops.
    #[command(mut_arg("device", |arg| arg.hide(true)))]
    Preset {
        /// Gesture preset to install.
        #[arg(value_parser = ["side-swipe"])]
        name: String,
        #[command(flatten)]
        scope: ScopeArgs,
    },
}

fn set_gesture(rules: &mut Vec<GestureRule>, rule: GestureRule) {
    rules.retain(|other| !(mouse_button(&other.button) == mouse_button(&rule.button) && other.direction.trim().eq_ignore_ascii_case(rule.direction.trim())
        && other.scope == rule.scope && same_app(&other.app, &rule.app)));
    rules.push(rule);
}

pub async fn gestures_command(dir: &Path, cmd: Option<GesturesCmd>) -> Result<()> {
    match cmd.unwrap_or(GesturesCmd::List) {
        GesturesCmd::List => {
            let settings = Settings::view(dir)?;
            if settings.input.gesture.is_empty() {
                println!("No gestures yet. Try `one-mk gestures preset side-swipe`, or `one-mk gestures add button4 left back`.");
            }
            for rule in settings.input.gesture {
                println!("Hold {} and swipe {} to {} ({} units){}.", rule.button, rule.direction, rule.to.describe(), rule.threshold_px, where_suffix(&rule.scope, &rule.app));
            }
            return Ok(());
        }
        GesturesCmd::Add { button, direction, to, threshold, scope } => {
            no_device(&scope)?;
            let button = keynames::key_name(mouse_button(&button).map_err(anyhow::Error::msg)?);
            let output = match to.as_slice() { [one] => To::One(one.clone()), _ => To::Many(to) };
            let rule = GestureRule { button, direction, to: output,
                threshold_px: threshold, scope: scope.scope(dir)?, app: scope.app()? };
            rule.compile(remap::Target { os: os::local(), swap: false }).map_err(anyhow::Error::msg)?;
            let paired = paired(dir);
            let notes = match &rule.to {
                To::One(text) => system_notes(&rule.scope, &paired, &[text]),
                To::Many(_) => vec![],
            };
            Settings::update(dir, |settings| {
                let known = reserved_gestures(&settings.input, &paired);
                set_gesture(&mut settings.input.gesture, rule);
                match reserved_gestures(&settings.input, &paired).into_iter().find(|problem| !known.contains(problem)) {
                    Some(problem) => bail!("{problem}; the gesture was not saved"),
                    None => Ok(()),
                }
            })?;
            println!("Gesture saved. A short click still clicks normally; press Esc to cancel a swipe.");
            print_system_notes(notes);
            run_note(dir);
        }
        GesturesCmd::Remove { button, direction, scope } => {
            no_device(&scope)?;
            let button = mouse_button(&button).map_err(anyhow::Error::msg)?;
            let app = scope.app()?;
            let scope = scope.selected(dir)?;
            let count = Settings::update(dir, |settings| {
                let rules = &mut settings.input.gesture;
                let before = rules.len();
                rules.retain(|rule| !(mouse_button(&rule.button) == Ok(button) && rule.direction.trim().eq_ignore_ascii_case(&direction) && rule.scope == scope && same_app(&rule.app, &app)));
                Ok(before - rules.len())
            })?;
            if count == 0 { bail!("no matching gesture; use `one-mk gestures list` to see the configured buttons, directions and scopes"); }
            println!("Removed {count} gesture(s).");
        }
        GesturesCmd::Clear { scope } => {
            no_device(&scope)?;
            let app = scope.app()?;
            let scope = scope.selected(dir)?;
            Settings::update(dir, |settings| { settings.input.gesture.retain(|rule| rule.scope != scope || !same_app(&rule.app, &app)); Ok(()) })?;
            println!("Gestures cleared in this scope.");
        }
        GesturesCmd::Preset { scope, .. } => {
            no_device(&scope)?;
            let app = scope.app()?;
            let scope = scope.scope(dir)?;
            let paired = paired(dir);
            Settings::update(dir, |settings| {
                // As `gestures add` does: not saved if autoscroll's toggle
                // button is button4 (the preset would be skipped for it).
                let known = reserved_gestures(&settings.input, &paired);
                for (direction, output) in [("left", "os:previous-desktop"), ("right", "os:next-desktop")] {
                    set_gesture(&mut settings.input.gesture, GestureRule { button: "button4".into(), direction: direction.into(),
                        to: To::One(output.into()), threshold_px: 60.0, scope: scope.clone(), app: app.clone() });
                }
                match reserved_gestures(&settings.input, &paired).into_iter().find(|problem| !known.contains(problem)) {
                    Some(problem) => bail!("{problem}; the preset was not saved"),
                    None => Ok(()),
                }
            })?;
            println!("Hold the back side button and swipe left or right to change desktops on the other computer.");
        }
    }
    settings::apply(dir).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removal_accepts_capitalized_config_direction() {
        use clap::Parser;
        #[derive(clap::Parser)]
        struct Gestures {
            #[command(subcommand)]
            command: GesturesCmd,
        }
        assert!(Gestures::try_parse_from(["gestures", "remove", "button4", "Left"]).is_ok());
    }

    #[test]
    fn unsafe_and_device_mouse_modes_warn_and_are_ignored() {
        for button in ["button1", "button2"] {
            let mut settings = TuningSettings::default();
            settings.autoscroll.base.enabled = Some(true);
            settings.autoscroll.base.button = Some(button.into());
            settings.gesture.push(GestureRule { button: button.into(), direction: "left".into(),
                to: To::One("back".into()), threshold_px: 60.0, scope: Scope::Global, app: None });
            let facts = PeerFacts { name: "pc", id: "", os: os::LINUX, apps: &[], device: None };
            let resolved = settings.resolve(&facts, false);
            assert!(resolved.autoscroll.is_none());
            assert!(resolved.gestures.is_empty());
            assert!(settings.check(&[]).iter().any(|warning| warning.contains("primary")));
        }
        let settings: Settings = toml::from_str("[input.autoscroll]\nenabled=true\nscope={device='usb/1234:5678//test'}\n[[input.gesture]]\nbutton='button4'\ndirection='left'\nto='back'\nscope={device='usb/1234:5678//test'}\n").unwrap();
        let warnings = settings.input.check(&[]);
        assert!(warnings.iter().any(|warning| warning.contains("device") && warning.contains("autoscroll") && warning.contains("ignored")));
        assert!(warnings.iter().any(|warning| warning.contains("device") && warning.contains("gesture") && warning.contains("ignored")));
        let fingerprint = "usb/1234:5678//test".parse().unwrap();
        for device in [None, Some(&fingerprint)] {
            let resolved = settings.input.resolve(&PeerFacts { name: "pc", id: "", os: os::LINUX, apps: &[], device }, false);
            assert!(resolved.autoscroll.is_none());
            assert!(resolved.gestures.is_empty());
        }
    }

    #[test]
    fn inherited_autoscroll_button_does_not_warn_about_middle_mapping() {
        let mut settings = TuningSettings::default();
        settings.autoscroll.base.button = Some("button4".into());
        settings.autoscroll.rule.push(AutoscrollRule { enabled: Some(true), scope: Scope::Os(os::WINDOWS), ..Default::default() });
        settings.button.push(ButtonRule { from: ButtonFrom::Number(3), to: To::One("back".into()), scope: Scope::Global, app: None });
        assert!(!settings.check(&[]).iter().any(|warning| warning.contains("autoscroll takes precedence")));
    }

    #[test]
    fn system_gestures_emit_target_shortcuts_and_check_resolves_the_peer_os() {
        let input: TuningSettings = toml::from_str(r#"
[[gesture]]
button = "button4"
direction = "left"
to = "os:previous-desktop"
scope = { peer = "Studio" }
[[gesture]]
button = "button5"
direction = "right"
to = "os:app-launcher"
scope = { peer = "Studio" }
[[keymap.rule]]
from = "a"
to = "os:app-launcher"
scope = { peer = "Studio" }
[[button]]
from = 3
to = "os:app-launcher"
scope = { os = "mac" }
"#).unwrap();
        assert!(input.check(&[("Studio".into(), "12345678".into(), os::MACOS)]).is_empty());
        let peer = PeerFacts { name: "Studio", id: "12345678", os: os::MACOS, apps: &[], device: None };
        let resolved = input.resolve(&peer, true);
        for source in ["gesture: button5", "a:", "button3"] {
            assert!(resolved.warnings.iter().any(|p| p.contains(source) && p.ends_with("macOS and sends nothing")), "{:?}", resolved.warnings);
        }
        assert_eq!(resolved.gestures.len(), 2);
        let mut mapper = remap::Mapper::new(resolved.keymap);
        let mut keys = Vec::new();
        for gesture in resolved.gestures {
            mapper.pulse_binding(&gesture.output, &mut |event| match event {
                remap::Emit::Key { code, down } => keys.push((one_mk_proto::swap_ctrl_meta(code), down)),
                _ => panic!("the gesture emits only keystrokes"),
            });
        }
        assert_eq!(keys, [(29, true), (105, true), (105, false), (29, false)]);
    }

    #[test]
    fn linux_side_swipes_keep_the_legacy_workspace_chord() {
        for (direction, code, old, new) in [
            ("left", 105, "desktop-left", "os:previous-desktop"),
            ("right", 106, "desktop-right", "os:next-desktop"),
        ] {
            for output in [old, new] {
                let rule = GestureRule { button: "button4".into(), direction: direction.into(), to: To::One(output.into()), threshold_px: 60.0, scope: Scope::Global, app: None };
                let (gesture, warnings) = rule.compile(remap::Target { os: os::LINUX, swap: false }).unwrap();
                assert!(warnings.is_empty());
                let mut keys = Vec::new();
                remap::Mapper::new(Keymap::default()).pulse_binding(&gesture.output, &mut |event| {
                    if let remap::Emit::Key { code, down } = event { keys.push((code, down)); }
                });
                assert_eq!(keys, [(29, true), (56, true), (code, true), (code, false), (29, false), (56, false)], "{output}");
            }
        }
    }

    #[test]
    fn existing_settings_migrate_without_changing_old_rules() {
        let previous = "[input.keymap]\ntap_ms=180\n[[input.button]]\nfrom=4\nto='back'\n[[input.pointer]]\nspeed=1.5\n";
        let mut settings: Settings = toml::from_str(previous).unwrap();
        assert!(settings.input.autoscroll.is_empty());
        assert!(settings.input.gesture.is_empty());
        let old = settings.input.clone();
        settings.input.autoscroll.base.enabled = Some(true);
        settings.input.autoscroll.rule.push(AutoscrollRule { speed: Some(2.0), scope: Scope::Os(os::WINDOWS), ..Default::default() });
        settings.input.gesture.push(GestureRule { button: "button4".into(), direction: "left".into(), to: To::One("back".into()), threshold_px: 60.0, scope: Scope::Global, app: None });
        let saved = toml::to_string(&settings).unwrap();
        let restored: Settings = toml::from_str(&saved).unwrap();
        assert_eq!(settings.input, restored.input);
        assert_eq!(old.keymap, restored.input.keymap);
        assert_eq!(old.button, restored.input.button);
        assert_eq!(old.pointer, restored.input.pointer);
        let resolved = restored.input.resolve(&PeerFacts { name: "win", id: "", os: os::WINDOWS, apps: &[], device: None }, false);
        assert_eq!(resolved.autoscroll.unwrap().speed, 2.0);
    }

    #[test]
    fn scope_selection_permissions_and_conflicts_include_gestures() {
        let mut settings = TuningSettings::default();
        settings.autoscroll.base.enabled = Some(true);
        settings.gesture.push(GestureRule { button: "middle".into(), direction: "left".into(), to: To::One("run: echo hello".into()), threshold_px: 60.0, scope: Scope::Global, app: Some("code*".into()) });
        assert!(settings.runs_commands());
        assert!(settings.uses_apps());
        let facts = PeerFacts { name: "pc", id: "", os: os::LINUX, apps: &["code".into()], device: None };
        assert_eq!(settings.apps_for(&facts), ["code*"]);
        let resolved = settings.resolve(&facts, false);
        assert!(resolved.gestures.is_empty());
        assert!(resolved.warnings.iter().any(|warning| warning.contains("reserved by autoscroll")));
        settings.gesture[0].app = None;
        assert!(settings.check(&[]).iter().any(|warning| warning.contains("reserved by autoscroll")));
        settings.gesture[0].app = Some("code*".into());
        settings.autoscroll.rule.push(AutoscrollRule { enabled: Some(false), app: Some("code*".into()), ..Default::default() });
        let resolved = settings.resolve(&facts, false);
        assert!(resolved.autoscroll.is_none());
        assert_eq!(resolved.gestures.len(), 1);
    }

    #[test]
    fn preset_replaces_aliases_and_case_insensitive_directions() {
        let mut rules = vec![GestureRule { button: "middle".into(), direction: " LEFT ".into(),
            to: To::One("back".into()), threshold_px: 60.0, scope: Scope::Global, app: None }];
        set_gesture(&mut rules, GestureRule { button: "button3".into(), direction: "left".into(),
            to: To::One("forward".into()), threshold_px: 80.0, scope: Scope::Global, app: None });
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].threshold_px, 80.0);
    }

    #[test]
    fn hostile_mouse_rules_and_parsers_never_panic() {
        let mut seed = 5139u64;
        let mut random = || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1); seed };
        for _ in 0..2000 {
            let text: String = (0..random() % 64).map(|_| char::from_u32((random() % 0x110000) as u32).unwrap_or('\0')).collect();
            let _ = mouse_button(&text);
            let _ = toml::from_str::<Settings>(&text);
            let _ = text.parse::<one_mk_proto::gesture::Direction>();
            let rule = GestureRule { button: text.clone(), direction: text.clone(), to: To::One(text),
                threshold_px: f64::from_bits(random()), scope: Scope::Global, app: None };
            let _ = rule.compile(remap::Target { os: os::LINUX, swap: false });
            let _ = AutoscrollRule { speed: Some(f64::from_bits(random())), ..Default::default() }.validate();
        }
    }
    fn dir_with(tag: &str, text: &str) -> std::path::PathBuf {
        let dir = crate::testdir::scratch(tag);
        std::fs::write(dir.join("settings.toml"), text).unwrap();
        dir
    }

    /// The side-swipe preset is refused, like `gestures add`, when its
    /// button is autoscroll's toggle: it would be skipped there.
    #[tokio::test]
    async fn the_gesture_preset_respects_autoscroll_conflicts() {
        let dir = dir_with("preset-conflict", "[input.autoscroll]\nenabled = true\nbutton = \"button4\"\n");
        let preset = || Some(GesturesCmd::Preset { name: "side-swipe".into(), scope: ScopeArgs::default() });
        let error = gestures_command(&dir, preset()).await.unwrap_err().to_string();
        assert!(error.contains("reserved by autoscroll") && error.contains("not saved"), "{error}");
        assert!(Settings::load(&dir).unwrap().input.gesture.is_empty(), "nothing was saved");
        // Without the conflict it installs.
        let dir = dir_with("preset-fine", "");
        gestures_command(&dir, preset()).await.unwrap();
        assert_eq!(Settings::load(&dir).unwrap().input.gesture.len(), 2);
    }

    /// Turning autoscroll off or resetting it also reaches a table for the
    /// same place written by hand, which would keep it on.
    #[tokio::test]
    async fn autoscroll_edits_reach_a_same_scope_table() {
        let text = "[input.autoscroll]\nspeed = 2.0\n[[input.autoscroll.rule]]\nenabled = true\n";
        let facts = PeerFacts { name: "pc", id: "", os: os::LINUX, apps: &[], device: None };
        let resolved = |dir: &Path| Settings::load(dir).unwrap().input.resolve(&facts, false).autoscroll;
        let dir = dir_with("autoscroll-off", text);
        assert!(resolved(&dir).is_some());
        autoscroll_command(&dir, AutoscrollArgs { off: true, ..Default::default() }).await.unwrap();
        assert!(resolved(&dir).is_none(), "the hand-written table kept autoscroll on");
        assert_eq!(Settings::load(&dir).unwrap().input.autoscroll.base.speed, Some(2.0), "other settings stay");
        let dir = dir_with("autoscroll-reset", text);
        autoscroll_command(&dir, AutoscrollArgs { reset: true, ..Default::default() }).await.unwrap();
        assert!(resolved(&dir).is_none());
        assert!(Settings::load(&dir).unwrap().input.autoscroll.is_empty());
        // And turning it on, after it was written off by a table.
        let dir = dir_with("autoscroll-on", "[[input.autoscroll.rule]]\nenabled = false\n");
        autoscroll_command(&dir, AutoscrollArgs { on: true, ..Default::default() }).await.unwrap();
        assert!(resolved(&dir).is_some());
    }

}
