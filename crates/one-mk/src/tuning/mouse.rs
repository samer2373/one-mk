use super::*;

#[derive(clap::Args, Debug, Default)]
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
    #[arg(long)]
    pub speed: Option<f64>,
    /// Movement before scrolling starts, from 0 to 1000 pixels (default 8).
    #[arg(long)]
    pub dead_zone: Option<f64>,
    /// Scroll only up and down, or in both directions.
    #[arg(long, value_name = "on|off", value_parser = settings::parse_on_off)]
    pub vertical_only: Option<bool>,
    #[command(flatten)]
    pub scope: ScopeArgs,
}

pub async fn autoscroll_command(dir: &Path, args: AutoscrollArgs) -> Result<()> {
    let scope = args.scope.scope(dir)?;
    let app = args.scope.app()?;
    if !args.on && !args.off && args.button.is_none() && args.speed.is_none() && args.dead_zone.is_none() && args.vertical_only.is_none() {
        let settings = Settings::load(dir)?;
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
    let update = AutoscrollRule { enabled: if args.on { Some(true) } else if args.off { Some(false) } else { None },
        button: args.button, speed: args.speed, dead_zone: args.dead_zone, vertical_only: args.vertical_only,
        scope: scope.clone(), app: app.clone() };
    update.validate().map_err(anyhow::Error::msg)?;
    Settings::update(dir, |settings| {
        let auto = &mut settings.input.autoscroll;
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
        Ok(())
    })?;
    println!("Autoscroll settings saved. Click the toggle button again, press Esc, or leave the computer to stop scrolling.");
    settings::apply(dir).await
}

#[derive(clap::Subcommand, Debug)]
pub enum GesturesCmd {
    /// Show held-button gestures.
    List,
    /// Hold BUTTON and swipe DIRECTION to type TO on the other computer.
    Add {
        button: String,
        #[arg(value_parser = ["up", "down", "left", "right"])]
        direction: String,
        /// A key, chord, quoted action, or several chords for a sequence.
        #[arg(required = true, num_args = 1..)]
        to: Vec<String>,
        /// Minimum swipe distance, from 1 to 10000 pixels (default 60).
        #[arg(long, default_value_t = 60.0)]
        threshold: f64,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Remove a button's gesture in this scope.
    Remove {
        button: String,
        #[arg(value_parser = ["up", "down", "left", "right"])]
        direction: String,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Remove all held-button gestures in this scope.
    Clear {
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Hold the back side button and swipe left or right between desktops.
    Preset {
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
            let settings = Settings::load(dir)?;
            if settings.input.gesture.is_empty() {
                println!("No gestures yet. Try `one-mk gestures preset side-swipe`, or `one-mk gestures add button4 left back`.");
            }
            for rule in settings.input.gesture {
                println!("Hold {} and swipe {} to {} ({} px){}.", rule.button, rule.direction, rule.to.describe(), rule.threshold_px, where_suffix(&rule.scope, &rule.app));
            }
            return Ok(());
        }
        GesturesCmd::Add { button, direction, to, threshold, scope } => {
            let button = keynames::key_name(mouse_button(&button).map_err(anyhow::Error::msg)?);
            let output = match to.as_slice() { [one] => To::One(one.clone()), _ => To::Many(to) };
            let rule = GestureRule { button, direction, to: output,
                threshold_px: threshold, scope: scope.scope(dir)?, app: scope.app()? };
            rule.compile(remap::Target { os: os::local(), swap: false }).map_err(anyhow::Error::msg)?;
            Settings::update(dir, |settings| { set_gesture(&mut settings.input.gesture, rule); Ok(()) })?;
            println!("Gesture saved. A short click still clicks normally; press Esc to cancel a swipe.");
        }
        GesturesCmd::Remove { button, direction, scope } => {
            let button = mouse_button(&button).map_err(anyhow::Error::msg)?;
            let app = scope.app()?;
            let scope = scope.selected()?;
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
            let app = scope.app()?;
            let scope = scope.selected()?;
            Settings::update(dir, |settings| { settings.input.gesture.retain(|rule| rule.scope != scope || !same_app(&rule.app, &app)); Ok(()) })?;
            println!("Gestures cleared in this scope.");
        }
        GesturesCmd::Preset { scope, .. } => {
            let app = scope.app()?;
            let scope = scope.scope(dir)?;
            Settings::update(dir, |settings| {
                for direction in ["left", "right"] {
                    set_gesture(&mut settings.input.gesture, GestureRule { button: "button4".into(), direction: direction.into(),
                        to: To::One(format!("desktop-{direction}")), threshold_px: 60.0, scope: scope.clone(), app: app.clone() });
                }
                Ok(())
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
        let resolved = restored.input.resolve(&PeerFacts { name: "win", id: "", os: os::WINDOWS, apps: &[] }, false);
        assert_eq!(resolved.autoscroll.unwrap().speed, 2.0);
    }

    #[test]
    fn scope_selection_permissions_and_conflicts_include_gestures() {
        let mut settings = TuningSettings::default();
        settings.autoscroll.base.enabled = Some(true);
        settings.gesture.push(GestureRule { button: "middle".into(), direction: "left".into(), to: To::One("run: echo hello".into()), threshold_px: 60.0, scope: Scope::Global, app: Some("code*".into()) });
        assert!(settings.runs_commands());
        assert!(settings.uses_apps());
        let facts = PeerFacts { name: "pc", id: "", os: os::LINUX, apps: &["code".into()] };
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
}
