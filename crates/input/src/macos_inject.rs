fn keyboard_layout() -> Option<crate::typing::KeyboardLayout> { crate::typing::KeyboardLayout::native() }

#[cfg(test)]
mod typing_capability_tests {
    #[test]
    fn selected_native_backend_advertises_text_and_character_holds() {
        let keyboard = super::keyboard_layout().expect("native backend must advertise a keyboard even before layout discovery");
        assert!(keyboard.can_inject());
        assert!(keyboard.native_text());
    }
}

fn inject_thread(
    tx: mpsc::UnboundedSender<Event>,
    rx: mpsc::UnboundedReceiver<Inject>,
    trusted: Arc<AtomicBool>,
    controlled: Arc<Controlled>,
) {
    let Some(source) = CGEventSource::new(CGEventSourceStateID::HIDSystemState) else {
        let _ = tx.send(Event::Error("Other computers can't control this Mac: couldn't create an input source.".into()));
        return;
    };
    CGEventSource::set_user_data(Some(&source), OUR_EVENTS);
    // By default, posting an event mutes the local mouse and keyboard for
    // a quarter second. Don't.
    CGEventSource::set_local_events_suppression_interval(Some(&source), 0.0);
    // A small runtime of its own, for the key repeat timer.
    let rt = match tokio::runtime::Builder::new_current_thread().enable_time().build() {
        Ok(rt) => rt,
        Err(e) => {
            let _ = tx.send(Event::Error(format!("Other computers can't control this Mac: {e}")));
            return;
        }
    };
    let inj = Injector::new(source);
    tracing::info!("input injection ready");
    rt.block_on(inject_loop(inj, rx, trusted, controlled));
}

/// A number from the global preferences domain (`defaults read -g key`).
fn global_pref(key: &'static str) -> Option<i64> {
    let key = CFString::from_static_str(key);
    // SAFETY: an immutable CF constant.
    let app = unsafe { kCFPreferencesAnyApplication };
    CFPreferencesCopyAppValue(&key, app)?.downcast_ref::<CFNumber>()?.as_i64()
}

/// The user's key repeat settings (System Settings > Keyboard).
fn repeat_setting() -> Option<(Duration, Duration)> {
    logic::repeat_timing(global_pref("InitialKeyRepeat"), global_pref("KeyRepeat"))
}

async fn inject_loop(
    mut inj: Injector,
    mut rx: mpsc::UnboundedReceiver<Inject>,
    trusted: Arc<AtomicBool>,
    controlled: Arc<Controlled>,
) {
    let mut warned = false;
    let mut batch = Vec::new();
    // CGEventPost doesn't auto-repeat; held keys and native characters repeat here.
    let mut repeat = KeyRepeat::new(repeat_setting());
    let mut settings_read = Instant::now();
    let mut text = crate::typing::NativeText::default();
    loop {
        let deadline = repeat.deadline();
        let wake = tokio::time::Instant::from_std(deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(3600)));
        tokio::select! {
            msg = rx.recv() => {
                let Some(first) = msg else { return };
                // Everything already queued, without the moves that later
                // ones make stale: a burst costs one move, not latency.
                drain_coalesced(first, &mut rx, &mut batch);
                if !trusted.load(Ordering::Relaxed) {
                    if !warned {
                        tracing::warn!("ignoring input from peers until Accessibility permission is granted");
                        warned = true;
                    }
                    controlled.apply(&mut batch);
                    batch.clear();
                    repeat.clear();
                    text.clear();
                    continue;
                }
                warned = false;
                let now = Instant::now();
                for m in batch.drain(..) {
                    if let Inject::Controlled(active) = m {
                        let was = controlled.get();
                        controlled.set(active);
                        if !active { repeat.clear(); }
                        if !was && active { let _ = CGDisplayShowCursor(kCGNullDirectDisplay); }
                    }
                    if let Inject::Key { code, down } | Inject::CharacterKey { code, down, .. } = m {
                        if down && is_repeatable(code) && now.duration_since(settings_read) > Duration::from_secs(5) {
                            repeat.set_timing(repeat_setting());
                            settings_read = now;
                        }
                        if !repeat.key(code, down, now) {
                            continue;
                        }
                    }
                    for message in text.input(m) { inj.apply(message); }
                }
            }
            _ = tokio::time::sleep_until(wake), if deadline.is_some() => {
                if let Some(code) = repeat.poll(Instant::now()) {
                    if let Some(messages) = text.repeat(code) {
                        for message in messages { inj.apply(message); }
                    } else { inj.repeat_key(code); }
                }
            }
        }
    }
}

struct Injector {
    source: CFRetained<CGEventSource>,
    pos: CGPoint,
    /// The displays, for keeping relative moves on them; read again when a
    /// move would leave them (they may have changed), at most once a second.
    desktop: Option<Desktop>,
    desktop_read: Instant,
    /// Held buttons, bit n = macOS button number n.
    buttons: u32,
    mods: logic::Modifiers,
    clicks: logic::ClickCounter,
    smooth: logic::Accum,
    wheel: logic::Accum,
    epoch: Instant,
}

impl Injector {
    fn new(source: CFRetained<CGEventSource>) -> Self {
        // Start from wherever the cursor is.
        let pos = CGEvent::new(None).map_or(CGPoint::new(0.0, 0.0), |e| CGEvent::location(Some(&e)));
        Self {
            source,
            pos,
            desktop: read_desktop(),
            desktop_read: Instant::now(),
            buttons: 0,
            mods: logic::Modifiers::default(),
            clicks: logic::ClickCounter::new(DOUBLE_CLICK_SECS, DOUBLE_CLICK_SLOP),
            smooth: logic::Accum::default(),
            wheel: logic::Accum::default(),
            epoch: Instant::now(),
        }
    }

    fn post(&self, ev: &CGEvent, extra_flags: u64) {
        CGEvent::set_flags(Some(ev), CGEventFlags(self.mods.flags() | extra_flags));
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
    }

    fn mouse_event(&self, ty: u32, button: u32) -> Option<CFRetained<CGEvent>> {
        let ev = CGEvent::new_mouse_event(Some(&self.source), CGEventType(ty), self.pos, CGMouseButton(button))?;
        if button >= 2 {
            CGEvent::set_integer_value_field(Some(&ev), CGEventField::MouseEventButtonNumber, i64::from(button));
        }
        Some(ev)
    }

    /// A move to `self.pos` that moved by `(dx, dy)`.
    fn post_move(&self, dx: f64, dy: f64) {
        // With a button held, apps expect drags, not moves
        // (selection, drag and drop).
        let held = (0..32).find(|n| self.buttons & (1 << n) != 0);
        let (ty, button) = match held {
            Some(n) => (logic::mouse_event_types(n).2, n),
            None => (CGEventType::MouseMoved.0, 0),
        };
        let Some(ev) = self.mouse_event(ty, button) else { return };
        CGEvent::set_double_value_field(Some(&ev), CGEventField::MouseEventDeltaX, dx);
        CGEvent::set_double_value_field(Some(&ev), CGEventField::MouseEventDeltaY, dy);
        self.post(&ev, 0);
    }

    fn apply(&mut self, msg: Inject) {
        match msg {
            Inject::MoveTo(p) => {
                let (dx, dy) = (p.x - self.pos.x, p.y - self.pos.y);
                self.pos = CGPoint::new(p.x, p.y);
                self.post_move(dx, dy);
            }
            // Games read the delta fields. The location is tracked here and
            // kept on the displays, as macOS keeps the cursor, so later
            // absolute moves start from the truth: reading it back right
            // after the (asynchronous) post could return the old one.
            Inject::MoveBy { dx, dy } => {
                let (dx, dy) = (f64::from(dx), f64::from(dy));
                let to = Pos::new(self.pos.x + dx, self.pos.y + dy);
                let off = self.desktop.as_ref().is_none_or(|d| d.screen_at(to).is_none());
                if off && self.desktop_read.elapsed() > Duration::from_secs(1) {
                    self.desktop = read_desktop().or(self.desktop.take());
                    self.desktop_read = Instant::now();
                }
                let p = logic::moved_by(self.desktop.as_ref(), Pos::new(self.pos.x, self.pos.y), dx, dy);
                self.pos = CGPoint::new(p.x, p.y);
                self.post_move(dx, dy);
            }
            Inject::Button { button, down } => {
                let Some(n) = logic::evdev_to_mac_button(button) else {
                    tracing::debug!(button, "unsupported mouse button");
                    return;
                };
                let (down_ty, up_ty, _) = logic::mouse_event_types(n);
                let count = if down {
                    let t = self.epoch.elapsed().as_secs_f64();
                    self.clicks.press(n, t, Pos::new(self.pos.x, self.pos.y))
                } else {
                    self.clicks.release(n)
                };
                let Some(ev) = self.mouse_event(if down { down_ty } else { up_ty }, n) else { return };
                CGEvent::set_integer_value_field(Some(&ev), CGEventField::MouseEventClickState, count);
                if down {
                    self.buttons |= 1 << n;
                } else {
                    self.buttons &= !(1 << n);
                }
                self.post(&ev, 0);
            }
            Inject::Scroll { dx, dy } => {
                let (v, h) = logic::wire_to_mac_scroll(dx, dy);
                let (h, v) = self.smooth.take(h, v);
                self.scroll(CGScrollEventUnit::Pixel, v, h);
            }
            Inject::ScrollDiscrete { dx, dy } => {
                let (v, h) = logic::wire_discrete_to_mac_lines(dx, dy);
                let (h, v) = self.wheel.take(h, v);
                self.scroll(CGScrollEventUnit::Line, v, h);
            }
            Inject::ScrollStop => self.smooth.reset(),
            // Handled by the injection loop.
            Inject::Controlled(_) => {}
            Inject::Text { text } => {
                let text: Vec<_> = text.as_str().encode_utf16().collect();
                let (Some(press), Some(release)) = (
                    CGEvent::new_keyboard_event(Some(&self.source), 0, true),
                    CGEvent::new_keyboard_event(Some(&self.source), 0, false),
                ) else { return };
                for event in [press, release] {
                    unsafe { CGEvent::keyboard_set_unicode_string(Some(&event), text.len() as u64, text.as_ptr()) };
                    self.post(&event, 0);
                }
            }
            Inject::Key { code, down } | Inject::CharacterKey { code, down, .. } => {
                // Each posted Caps Lock press toggles it; a repeat must not.
                if self.mods.is_caps_repeat(code, down) {
                    return;
                }
                self.mods.update(code, down);
                if code == KEY_CAPSLOCK && !down {
                    return; // a toggle: one event per press
                }
                // Media keys are system-defined events; key events for the
                // volume and brightness keys do nothing.
                if let Some(nx) = mediakeys::to_mac_nx(code) {
                    self.media_key(nx, down, false);
                    return;
                }
                if mediakeys::is_media(code) {
                    tracing::debug!(code, "no Mac media key for this key code");
                    return;
                }
                let Some(kc) = logic::evdev_to_mac(code) else {
                    tracing::debug!(code, "no Mac key for this key code");
                    return;
                };
                let Some(ev) = CGEvent::new_keyboard_event(Some(&self.source), kc, down) else { return };
                if logic::is_mac_modifier(kc) {
                    CGEvent::set_type(Some(&ev), CGEventType::FlagsChanged);
                }
                self.post(&ev, logic::key_extra_flags(kc));
            }
        }
    }

    /// Another "down" for a held key, marked as auto-repeat like a real one.
    fn repeat_key(&self, code: u32) {
        if let Some(nx) = mediakeys::to_mac_nx(code) {
            self.media_key(nx, true, true);
            return;
        }
        let Some(kc) = logic::evdev_to_mac(code) else { return };
        let Some(ev) = CGEvent::new_keyboard_event(Some(&self.source), kc, true) else { return };
        CGEvent::set_integer_value_field(Some(&ev), CGEventField::KeyboardEventAutorepeat, 1);
        self.post(&ev, logic::key_extra_flags(kc));
    }

    /// Posts a media key press or release (`NX_SYSDEFINED`, subtype 8). In
    /// an autorelease pool: this thread has none, and AppKit autoreleases.
    fn media_key(&self, nx: u8, down: bool, repeat: bool) {
        objc2::rc::autoreleasepool(|_| self.post_media_key(nx, down, repeat));
    }

    fn post_media_key(&self, nx: u8, down: bool, repeat: bool) {
        let state = if down { 0xA00 } else { 0xB00 };
        let Some(ns) = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            NSEventType::SystemDefined,
            CGPoint::new(0.0, 0.0),
            NSEventModifierFlags(state),
            0.0,
            0,
            None,
            logic::NX_SUBTYPE_AUX_CONTROL_BUTTONS,
            logic::media_data1(nx, down, repeat) as isize,
            -1,
        ) else {
            return;
        };
        let Some(ev) = ns.CGEvent() else { return };
        // Our own tap must let it through, and see it is not local input.
        CGEvent::set_integer_value_field(Some(&ev), CGEventField::EventSourceUserData, OUR_EVENTS);
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&ev));
    }

    fn scroll(&self, unit: CGScrollEventUnit, vertical: i32, horizontal: i32) {
        if vertical == 0 && horizontal == 0 {
            return;
        }
        if let Some(ev) = CGEvent::new_scroll_wheel_event2(Some(&self.source), unit, 2, vertical, horizontal, 0) {
            self.post(&ev, 0);
        }
    }
}
