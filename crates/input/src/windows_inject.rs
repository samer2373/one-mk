fn keyboard_layout() -> Option<crate::typing::KeyboardLayout> { crate::typing::KeyboardLayout::native() }


fn inject_thread(
    tx: mpsc::UnboundedSender<Event>,
    rx: mpsc::UnboundedReceiver<Inject>,
    shared: Arc<Mutex<Space>>,
    controlled: Arc<Controlled>,
) {
    set_thread_dpi_awareness();
    // A small runtime of its own, for the key repeat timer.
    let rt = match tokio::runtime::Builder::new_current_thread().enable_time().build() {
        Ok(rt) => rt,
        Err(e) => {
            let _ = tx.send(Event::Error(format!("Other computers can't control this one: {e}")));
            return;
        }
    };
    tracing::info!("input injection ready");
    rt.block_on(inject_loop(tx, rx, shared, controlled));
}

/// The user's key repeat delay and rate (Control Panel > Keyboard).
fn repeat_setting() -> Option<(Duration, Duration)> {
    let (mut delay, mut speed) = (1u32, 31u32);
    // SAFETY: both actions write one u32 to the valid out-pointer.
    unsafe {
        SystemParametersInfoW(SPI_GETKEYBOARDDELAY, 0, (&mut delay as *mut u32).cast(), 0);
        SystemParametersInfoW(SPI_GETKEYBOARDSPEED, 0, (&mut speed as *mut u32).cast(), 0);
    }
    Some(repeat_timing(delay, speed))
}

async fn inject_loop(
    tx: mpsc::UnboundedSender<Event>,
    mut rx: mpsc::UnboundedReceiver<Inject>,
    shared: Arc<Mutex<Space>>,
    controlled: Arc<Controlled>,
) {
    let mut scroll = SmoothScroll::default();
    let mut rel = (0.0, 0.0);
    let mut inputs: Vec<INPUT> = Vec::new();
    let mut batch: Vec<Inject> = Vec::new();
    // SendInput doesn't auto-repeat; held keys and native characters repeat here.
    let mut repeat = KeyRepeat::new(repeat_setting());
    let mut failing = false;
    let mut text = crate::typing::NativeText::default();
    loop {
        let deadline = repeat.deadline();
        let wake = tokio::time::Instant::from_std(deadline.unwrap_or_else(|| std::time::Instant::now() + Duration::from_secs(3600)));
        tokio::select! {
            msg = rx.recv() => {
                let Some(first) = msg else { return };
                // Everything already queued goes out in one SendInput call,
                // without the moves that later ones make stale.
                drain_coalesced(first, &mut rx, &mut batch);
                let now = std::time::Instant::now();
                let Ok(space) = shared.lock() else { return };
                for m in batch.drain(..) {
                    if let Inject::Controlled(active) = m {
                        controlled.set(active);
                        if !active { repeat.clear(); }
                    }
                    if let Inject::Key { code, down } | Inject::CharacterKey { code, down, .. } = m {
                        if down && is_repeatable(code) {
                            repeat.set_timing(repeat_setting());
                        }
                        if !repeat.key(code, down, now) {
                            continue;
                        }
                    }
                    for message in text.input(m) { build(&mut inputs, message, &space, &mut scroll, &mut rel); }
                }
            }
            _ = tokio::time::sleep_until(wake), if deadline.is_some() => {
                let Some(code) = repeat.poll(std::time::Instant::now()) else { continue };
                let Ok(space) = shared.lock() else { return };
                if let Some(messages) = text.repeat(code) {
                    for message in messages { build(&mut inputs, message, &space, &mut scroll, &mut rel); }
                } else {
                    build(&mut inputs, Inject::Key { code, down: true }, &space, &mut scroll, &mut rel);
                }
            }
        }
        send_inputs(&mut inputs, &tx, &mut failing);
    }
}

fn send_inputs(inputs: &mut Vec<INPUT>, tx: &mpsc::UnboundedSender<Event>, failing: &mut bool) {
    if inputs.is_empty() {
        return;
    }
    // SAFETY: `inputs` is a valid slice of initialized INPUTs.
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    if (sent as usize) < inputs.len() {
        let err = unsafe { GetLastError() };
        if sent > 0 {
            let last = &inputs[sent as usize - 1];
            if last.r#type == INPUT_KEYBOARD {
                let keyboard = unsafe { last.Anonymous.ki };
                if keyboard.dwFlags & (KEYEVENTF_UNICODE | KEYEVENTF_KEYUP) == KEYEVENTF_UNICODE {
                    let release = key(0, keyboard.wScan, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP);
                    unsafe { SendInput(1, &release, size_of::<INPUT>() as i32) };
                }
            }
        }
        if !*failing {
            tracing::warn!(sent, total = inputs.len(), err, "SendInput failed");
            let _ = tx.send(Event::Error(
                "Windows blocked input from another computer. This happens on the lock screen, \
                 on administrator (UAC) prompts, and in programs running as administrator."
                    .into(),
            ));
            *failing = true;
        }
    } else {
        *failing = false;
    }
    inputs.clear();
}

pub(crate) fn mouse(dx: i32, dy: i32, data: u32, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx, dy, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: OUR_INPUT },
        },
    }
}

pub(crate) fn key(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: OUR_INPUT } },
    }
}

/// `rel` keeps the fraction of a physical pixel that relative moves (in
/// logical pixels) did not use yet, so scaling never drifts.
fn build(out: &mut Vec<INPUT>, m: Inject, space: &Space, scroll: &mut SmoothScroll, rel: &mut (f64, f64)) {
    const ABS_MOVE: u32 = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    match m {
        Inject::MoveTo(p) => {
            let (x, y) = space.to_phys(p);
            let (nx, ny) = space.normalize(x, y);
            out.push(mouse(nx, ny, 0, ABS_MOVE));
        }
        // A plain relative move: what games reading raw input see. (Windows
        // applies the pointer speed setting to it for the desktop cursor.)
        Inject::MoveBy { dx, dy } => {
            let fx = rel.0 + f64::from(dx) * space.scale();
            let fy = rel.1 + f64::from(dy) * space.scale();
            let (px, py) = (fx.round(), fy.round());
            *rel = (fx - px, fy - py);
            if px != 0.0 || py != 0.0 {
                out.push(mouse(px as i32, py as i32, 0, MOUSEEVENTF_MOVE));
            }
        }
        Inject::Button { button, down } => {
            let (flags, data) = match (button, down) {
                (BTN_LEFT, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                (BTN_LEFT, false) => (MOUSEEVENTF_LEFTUP, 0),
                (BTN_RIGHT, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                (BTN_RIGHT, false) => (MOUSEEVENTF_RIGHTUP, 0),
                (BTN_MIDDLE, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                (BTN_MIDDLE, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                (BTN_SIDE, true) => (MOUSEEVENTF_XDOWN, u32::from(XBUTTON1)),
                (BTN_SIDE, false) => (MOUSEEVENTF_XUP, u32::from(XBUTTON1)),
                (BTN_EXTRA, true) => (MOUSEEVENTF_XDOWN, u32::from(XBUTTON2)),
                (BTN_EXTRA, false) => (MOUSEEVENTF_XUP, u32::from(XBUTTON2)),
                _ => {
                    tracing::debug!(button, "no Windows equivalent for mouse button");
                    return;
                }
            };
            out.push(mouse(0, 0, data, flags));
        }
        Inject::ScrollDiscrete { dx, dy } => {
            let (h, v) = wire_to_wheel(dx, dy);
            push_wheel(out, h, v);
        }
        Inject::Scroll { dx, dy } => {
            let (h, v) = scroll.push(dx, dy);
            push_wheel(out, h, v);
        }
        Inject::ScrollStop => scroll.stop(),
        // Handled by the injection loop.
        Inject::Controlled(_) => {}
        Inject::Text { text } => {
            for (unit, down) in text.unicode_keys() {
                out.push(key(0, unit, KEYEVENTF_UNICODE | if down { 0 } else { KEYEVENTF_KEYUP }));
            }
        }
        Inject::Key { code, down } | Inject::CharacterKey { code, down, .. } => {
            let up = if down { 0 } else { KEYEVENTF_KEYUP };
            match evdev_to_win(code) {
                Some(WinKey::Scan { scan, extended }) => {
                    let ext = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
                    out.push(key(0, scan, KEYEVENTF_SCANCODE | ext | up));
                }
                Some(WinKey::Vk { vk, scan, extended }) => {
                    let ext = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
                    out.push(key(vk, scan, ext | up));
                }
                None => tracing::debug!(code, "no Windows equivalent for key"),
            }
        }
    }
}

/// Wheel amounts are signed but travel in the unsigned `mouseData`.
fn push_wheel(out: &mut Vec<INPUT>, h: i32, v: i32) {
    if v != 0 {
        out.push(mouse(0, 0, v as u32, MOUSEEVENTF_WHEEL));
    }
    if h != 0 {
        out.push(mouse(0, 0, h as u32, MOUSEEVENTF_HWHEEL));
    }
}

#[cfg(test)]
mod typing_capability_tests {
    #[test]
    fn selected_native_backend_advertises_text_and_character_holds() {
        let keyboard = super::keyboard_layout().expect("native backend must advertise a keyboard even before layout discovery");
        assert!(keyboard.can_inject());
        assert!(keyboard.native_text());
    }
}
