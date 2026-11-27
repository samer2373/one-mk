//! `one-mk input-devices detect`: which key or mouse button is that? The
//! command asks the running daemon (`Request::Detect`), which turns on the
//! backend's reporting of this computer's own presses (`CaptureCmd::Detect`),
//! answers with the first press that makes a trigger (see
//! [`one_mk_proto::detect`]) and turns it off again: when the press comes,
//! when the wait is over, when the command is stopped (it hangs up) or when
//! the daemon stops. Nothing is changed by detecting: the press still does
//! what it does. The answer is one line of JSON, which `--json` prints as
//! it is (the window reads it) and which is otherwise put in words.

use anyhow::{Result, bail};
use one_mk_proto::detect::{Detector, Found, LOCAL_UNSEEN, Step};
use one_mk_proto::keynames;
use serde_json::{Value, json};
use std::path::Path;
use tokio::sync::oneshot;
use tokio::time::Instant;

/// One detection in the daemon.
pub struct Session {
    /// Its number: the backend's reports carry it (`CaptureCmd::Detect`).
    pub id: u32,
    /// The backend reports this computer's own presses (`Event::LocalPress`),
    /// here and captured: input sent to another computer is not read too.
    pub local_presses: bool,
    reply: oneshot::Sender<String>,
    /// When the wait is over.
    pub until: Instant,
    seconds: u32,
    pub detector: Detector,
    epoch: std::time::Instant,
    /// Why this computer's own keys can't be seen (the Wayland portals): said with a timeout.
    unseen: Option<&'static str>,
}

/// How a detection ended.
#[derive(Debug, PartialEq)]
pub enum End {
    Found {
        found: Found,
        device: Option<(String, String)>,
        /// Why no rule for it can work here (`Found::unusable`, or this
        /// computer's backend can't apply one).
        unusable: Option<&'static str>,
    },
    Timeout,
    Cancelled,
    /// Another detection was asked for: this one gives way.
    Replaced,
    /// The daemon stops.
    Stopped,
}

impl Session {
    pub fn new(
        id: u32,
        reply: oneshot::Sender<String>,
        seconds: u32,
        keys_only: bool,
        hints_expected: bool,
        local_presses: bool,
        unseen: Option<&'static str>,
    ) -> Self {
        Session {
            id,
            local_presses,
            reply,
            until: Instant::now() + std::time::Duration::from_secs(u64::from(seconds)),
            seconds,
            detector: Detector::new(hints_expected, keys_only),
            epoch: std::time::Instant::now(),
            unseen,
        }
    }

    /// Milliseconds since it began, the detector's clock.
    pub fn ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// The next time it must be looked at: a press waiting for its device, or the end of the wait.
    pub fn deadline(&self) -> Instant {
        match self.detector.deadline() {
            Some(ms) => (Instant::from_std(self.epoch) + std::time::Duration::from_millis(ms)).min(self.until),
            None => self.until,
        }
    }

    /// Whoever asked has gone (the command was stopped, the window cancelled).
    pub fn abandoned(&self) -> bool {
        self.reply.is_closed()
    }

    /// Waits until whoever asked hangs up.
    pub async fn hung_up(&mut self) {
        self.reply.closed().await;
    }

    /// Sends the answer.
    pub fn finish(self, end: End) {
        let _ = self.reply.send(format!("{}\n", encode(&end, self.seconds, self.unseen)));
    }
}

/// A step of the detector, as an end (or none yet).
pub fn ended(step: Step) -> Option<End> {
    match step {
        Step::Wait => None,
        Step::Cancel => Some(End::Cancelled),
        Step::Found(found) => Some(End::Found { found, device: None, unusable: None }),
    }
}

/// The answer, as one line of JSON.
pub fn encode(end: &End, seconds: u32, unseen: Option<&str>) -> String {
    let v = match end {
        End::Found { found, device, unusable } => {
            let button = keynames::button_number(found.code);
            json!({
                "result": "found",
                "kind": if found.is_button() { "button" } else { "key" },
                "code": found.code,
                "name": keynames::key_name(found.code),
                "trigger": found.trigger().to_string(),
                "button": button,
                "shortcut": found.trigger().has_mods(),
                "device": device.as_ref().map(|(fingerprint, name)| json!({ "fingerprint": fingerprint, "name": name })),
                "away": found.away,
                "unusable": unusable.or(found.unusable()),
                "dropped": found.dropped_text(),
            })
        }
        End::Timeout => json!({ "result": "timeout", "seconds": seconds, "note": unseen }),
        End::Cancelled => json!({ "result": "cancelled" }),
        End::Replaced => json!({ "result": "replaced" }),
        End::Stopped => json!({ "result": "stopped" }),
    };
    v.to_string()
}

/// The window retains the write end of a pipe for as long as it lives.
pub async fn stdin_closed() {
    let (closed, wait) = oneshot::channel();
    // A detached thread allows the CLI to exit after a successful detection
    // even while the window still holds the pipe open.
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
        let _ = closed.send(());
    });
    let _ = wait.await;
}

/// `one-mk input-devices detect`.
pub async fn command(dir: &Path, seconds: u32, keys_only: bool, json: bool) -> Result<()> {
    let not_running = || {
        if json {
            println!("{}", json!({ "result": "not-running", "message": crate::control::NOT_RUNNING }));
        }
        anyhow::anyhow!("{}", crate::control::NOT_RUNNING)
    };
    let status = match crate::control::request(dir, crate::control::Request::Status).await? {
        Some(text) => one_mk_ctl::status::Status::decode(&text),
        None => return Err(not_running()),
    };
    let backend = status.as_ref().map_or("", |s| s.backend.as_str());
    let away = status.as_ref().is_some_and(|s| s.controlling.is_some());
    if !json {
        eprintln!("{}", waiting_text(backend, away, seconds, keys_only));
    }
    let reply = match crate::control::request(dir, crate::control::Request::Detect { timeout: seconds, keys_only }).await? {
        Some(text) if !text.trim().is_empty() => text,
        // It hung up without an answer: it stopped (or was killed) meanwhile.
        Some(_) => {
            let message = "one-mk stopped while waiting for a key; start it again with `one-mk run`";
            if json {
                println!("{}", json!({ "result": "stopped", "message": message }));
            }
            bail!("{message}");
        }
        None => return Err(not_running()),
    };
    let reply = reply.trim();
    if let Some(error) = reply.strip_prefix("error ") {
        // A one-mk from before this command was added.
        let message = if error.starts_with("unknown request") {
            "the running one-mk is older and can't detect keys: restart it (`one-mk stop`, then start it again)".to_owned()
        } else {
            error.to_owned()
        };
        if json {
            println!("{}", json!({ "result": "error", "message": message }));
        }
        bail!("{message}");
    }
    let v: Value = serde_json::from_str(reply).map_err(|e| {
        anyhow::anyhow!("the running one-mk answered something unexpected ({e}); is it an older version? Restart it")
    })?;
    if json {
        println!("{reply}");
    }
    match v["result"].as_str() {
        Some("found") => {
            if !json {
                print!("{}", found_text(&v));
            }
            Ok(())
        }
        Some("timeout") if json => bail!("nothing was pressed"),
        Some("timeout") => bail!("{}", timeout_text(&v)),
        Some("cancelled") => bail!("cancelled (Esc)"),
        Some("replaced") => bail!("another detection started (in the window, or another command), so this one stopped"),
        Some("stopped") => bail!("one-mk stopped while waiting for a key"),
        _ => bail!("the running one-mk answered something unexpected; restart it"),
    }
}

/// What the command says before it waits.
pub fn waiting_text(backend: &str, away: bool, seconds: u32, keys_only: bool) -> String {
    let local = one_mk_proto::detect::detect_support(backend);
    let what = if keys_only { "key" } else { "key or mouse button" };
    let mut out = format!("Press the {what} (Esc cancels; waiting {seconds} s)…");
    match local {
        Err(why) if why == LOCAL_UNSEEN && !away => out.push_str(
            "\nOn this desktop oneMK sees keys only while it controls another computer: move the pointer onto another \
             computer's screen (or use `one-mk switch`), then press it there.",
        ),
        Err(why) if why != LOCAL_UNSEEN => {
            let mut c = why.chars();
            let why: String = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
            out.push_str(&format!("\n{why}: only keys pressed while it controls another computer can be detected."));
        }
        _ => {}
    }
    out
}

/// What was found, in words, with the commands that change it.
pub fn found_text(v: &Value) -> String {
    let code = v["code"].as_u64().unwrap_or(0);
    let trigger = v["trigger"].as_str().unwrap_or("");
    // `f13` as F13, `play/pause` as Play/pause: as people write them.
    let label = |s: &str| -> String {
        let mut c = s.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
    };
    let mut out = match (v["kind"].as_str(), v["button"].as_u64()) {
        (Some("button"), Some(n)) => format!("Mouse button {n} ({})", keynames::key_name(code as u32)),
        (Some("button"), None) => "A mouse button past the eighth".to_owned(),
        _ if v["shortcut"].as_bool() == Some(true) => {
            let shortcut = trigger.split('+').map(|part| {
                part.strip_prefix("code:").map_or_else(|| label(part), |code| format!("Key code {code}"))
            }).collect::<Vec<_>>().join("+");
            format!("The shortcut {shortcut}")
        }
        _ => match v["name"].as_str() {
            Some(name) if !name.starts_with("code:") => format!("The key {} (code {code})", label(name)),
            _ => format!("Key code {code}"),
        },
    };
    let device = v["device"]["fingerprint"].as_str();
    if let Some(name) = v["device"]["name"].as_str() {
        out.push_str(&format!(", on {name}"));
    }
    out.push_str(".\n");
    if let Some(dropped) = v["dropped"].as_str() {
        out.push_str(&format!(
            "{} was held too: a button rule is for the button alone, so it is left out.\n",
            label(dropped)
        ));
    }
    if let Some(why) = v["unusable"].as_str() {
        out.push_str(&format!("It was detected, but it can't be changed on this system: {why}.\n"));
        return out;
    }
    let device_arg = device.map(|d| {
        #[cfg(not(windows))]
        let quoted = format!("'{}'", d.replace('\'', r"'\''"));
        #[cfg(windows)]
        let quoted = format!("'{}'", d.replace('\'', "''")); // PowerShell
        format!(" --device {quoted}")
    });
    match v["button"].as_u64().filter(|_| v["shortcut"].as_bool() != Some(true)) {
        Some(n) => {
            out.push_str(&format!("\nChange it with:\n  one-mk buttons map -- {n} TO\n"));
            if let Some(d) = &device_arg {
                out.push_str(&format!("Only on this mouse:\n  one-mk buttons map{d} -- {n} TO\n"));
            }
        }
        None => {
            out.push_str(&format!("\nChange it with:\n  one-mk keymap add -- {trigger} TO\n"));
            if let Some(d) = &device_arg {
                out.push_str(&format!("Only on this device:\n  one-mk keymap add{d} -- {trigger} TO\n"));
            }
        }
    }
    out.push_str("(TO: a key, a shortcut like ctrl+c, `none`, or an action like `switch next`.)\n");
    if v["device"].is_null() {
        out.push_str(
            "Which device it came from isn't known here, so a rule for it applies to every keyboard and mouse.\n",
        );
    }
    if v["away"].as_bool() == Some(true) {
        out.push_str("It was pressed while this computer controlled another one: rules change it there.\n");
    }
    out
}

fn timeout_text(v: &Value) -> String {
    let seconds = v["seconds"].as_u64().unwrap_or(0);
    match v["note"].as_str() {
        Some(note) if note == LOCAL_UNSEEN => format!(
            "nothing was pressed in {seconds} s ({note}: move the pointer onto another computer's screen, then press the key; \
             or name it yourself with `one-mk keymap add`)"
        ),
        Some(note) => format!("nothing was pressed in {seconds} s ({note})"),
        None => format!(
            "nothing was pressed in {seconds} s. Some keys never reach oneMK: keys a keyboard handles itself (often Fn), \
             and mouse buttons the system doesn't report as buttons (vendor buttons that need the maker's software)"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_proto::device::DeviceRef;

    fn found(code: u32, mods: u8) -> End {
        End::Found {
            found: Found { code, mods, device: Some(DeviceRef(3)), away: false, dropped: 0 },
            device: Some(("usb/046d:c52b//MX Master".into(), "MX Master".into())),
            unusable: None,
        }
    }

    #[test]
    fn answers_are_json_with_what_a_rule_needs() {
        let v: Value = serde_json::from_str(&encode(&found(0x116, 0), 30, None)).unwrap();
        assert_eq!(
            (v["result"].as_str(), v["kind"].as_str(), v["button"].as_u64(), v["trigger"].as_str()),
            (Some("found"), Some("button"), Some(7), Some("button7"))
        );
        assert_eq!(v["device"]["fingerprint"], "usb/046d:c52b//MX Master");
        let v: Value = serde_json::from_str(&encode(&found(15, keynames::LCTRL | keynames::RALT), 30, None)).unwrap();
        assert_eq!(
            (v["kind"].as_str(), v["trigger"].as_str(), v["shortcut"].as_bool()),
            (Some("key"), Some("ctrl+alt+tab"), Some(true))
        );
        let text = found_text(&v);
        assert!(text.starts_with("The shortcut Ctrl+Alt+Tab"), "{text}");
        assert!(text.contains("one-mk keymap add --device 'usb/046d:c52b//MX Master' -- ctrl+alt+tab TO"), "{text}");
        let v: Value = serde_json::from_str(&encode(&End::Timeout, 30, Some(LOCAL_UNSEEN))).unwrap();
        assert!(timeout_text(&v).contains("move the pointer onto another computer"));
        let v: Value = serde_json::from_str(&encode(
            &End::Found {
                found: Found { code: one_mk_proto::keynames::CODE_LIMIT, mods: 0, device: None, away: true, dropped: 0 },
                device: None,
                unusable: None,
            },
            5,
            None,
        ))
        .unwrap();
        assert!(v["unusable"].is_string() && v["device"].is_null());
        assert!(found_text(&v).contains("detected, but it can't be changed on this system"));
    }

    #[test]
    fn unnamed_keys_and_shell_sensitive_device_names_read_cleanly() {
        let v = json!({
            "kind": "key", "code": 432, "name": "code:432", "trigger": "code:432",
            "device": { "fingerprint": "usb/path/O'Brien $key \"quoted\"" }
        });
        let text = found_text(&v);
        assert!(text.starts_with("Key code 432.\n"), "{text}");
        #[cfg(not(windows))]
        assert!(text.contains(r#"--device 'usb/path/O'\''Brien $key "quoted"'"#), "{text}");
        #[cfg(windows)]
        assert!(text.contains(r#"--device 'usb/path/O''Brien $key "quoted"'"#), "{text}");
    }

    #[test]
    fn a_button_with_modifiers_is_offered_as_the_button_alone_and_says_so() {
        // Ctrl held + Back: `keymap add -- ctrl+button4` is refused, so the answer is button 4
        // for `buttons map`, which takes it, with the Ctrl said.
        let found = Found { code: 0x113, mods: 0, device: None, away: false, dropped: keynames::LCTRL };
        let v: Value = serde_json::from_str(&encode(&End::Found { found, device: None, unusable: None }, 30, None)).unwrap();
        assert_eq!((v["trigger"].as_str(), v["button"].as_u64(), v["dropped"].as_str()), (Some("button4"), Some(4), Some("ctrl")));
        let text = found_text(&v);
        assert!(text.contains("one-mk buttons map -- 4 TO") && !text.contains("keymap add"), "{text}");
        assert!(text.contains("Ctrl was held too"), "{text}");
    }

    #[test]
    fn a_press_this_system_cant_change_is_named_and_not_offered() {
        let found = Found { code: 0x115, mods: 0, device: None, away: false, dropped: 0 };
        let why = "oneMK forwards and remaps mouse buttons 1 to 5 here, and this one is further on";
        let v: Value = serde_json::from_str(&encode(&End::Found { found, device: None, unusable: Some(why) }, 30, None)).unwrap();
        let text = found_text(&v);
        assert!(text.starts_with("Mouse button 6") && text.contains(why), "{text}");
        assert!(!text.contains("buttons map"), "{text}");
    }

    #[test]
    fn the_wait_says_when_local_keys_cant_be_seen() {
        assert!(waiting_text("portal", false, 30, false).contains("only while it controls another computer"));
        assert!(!waiting_text("portal", true, 30, false).contains("only while"), "already controlling one");
        assert!(!waiting_text("x11", false, 30, false).contains('\n'));
        assert!(waiting_text("null", false, 5, false).contains("The null input backend"));
    }
}
