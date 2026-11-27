//! "Detect key" and "Detect button" on Keys & mouse: what `one-mk
//! input-devices detect --json` answers, read; and, where oneMK can't see
//! this computer's own keys (a Wayland desktop while the pointer is here) or
//! isn't running, the window's own key and mouse events instead.

use one_mk_proto::keynames;
use serde_json::Value;

/// A press, found (by oneMK or by the window).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Detected {
    /// Its evdev code, as the rules see it.
    pub code: u32,
    /// The modifier groups held with it (`keynames::CTRL` ...): a mouse
    /// button that sends a shortcut.
    pub groups: u8,
    /// Mouse button 1 to 8.
    pub button: Option<u32>,
    /// A mouse button (one past the eighth, too).
    pub is_button: bool,
    /// The device's fingerprint and name, where this computer can tell.
    pub device: Option<(String, String)>,
    /// Why no rule can use it.
    pub unusable: Option<String>,
}

/// How a detection came out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Found(Detected),
    /// Nothing was pressed in this many seconds (with why, where local keys can't be seen).
    Timeout(u64, Option<String>),
    Cancelled,
    /// oneMK isn't running, stopped meanwhile, or failed: what to say.
    Failed(String),
}

/// `one-mk input-devices detect --json`'s answer.
pub fn parse(text: &str) -> Outcome {
    let line = text.lines().rev().find(|l| l.trim_start().starts_with('{')).unwrap_or("");
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
        return Outcome::Failed("oneMK gave no answer. Try again, or choose the key from the list.".into());
    };
    match v["result"].as_str() {
        Some("found") => {
            let code = v["code"].as_u64().and_then(|c| u32::try_from(c).ok()).unwrap_or(0);
            let groups = v["trigger"].as_str().and_then(|t| t.parse::<keynames::Trigger>().ok()).map_or(0, |t| t.groups);
            let device = match (v["device"]["fingerprint"].as_str(), v["device"]["name"].as_str()) {
                (Some(fp), Some(name)) => Some((fp.to_owned(), name.to_owned())),
                _ => None,
            };
            Outcome::Found(Detected {
                code,
                groups,
                button: v["button"].as_u64().and_then(|n| u32::try_from(n).ok()),
                is_button: v["kind"].as_str() == Some("button"),
                device,
                unusable: v["unusable"].as_str().map(str::to_owned),
            })
        }
        Some("timeout") => Outcome::Timeout(v["seconds"].as_u64().unwrap_or(0), v["note"].as_str().map(str::to_owned)),
        Some("cancelled" | "replaced") => Outcome::Cancelled,
        Some("not-running") => Outcome::Failed("oneMK isn't running, so only keys this window sees can be detected.".into()),
        Some("stopped") => Outcome::Failed("oneMK stopped while waiting. Start it again, then try once more.".into()),
        _ => Outcome::Failed("oneMK answered something unexpected. Restart it, then try again.".into()),
    }
}

/// What the window's own mouse events say: egui's buttons as numbers.
pub fn pointer_button(b: egui::PointerButton) -> u32 {
    match b {
        egui::PointerButton::Primary => 1,
        egui::PointerButton::Secondary => 2,
        egui::PointerButton::Middle => 3,
        egui::PointerButton::Extra1 => 4,
        egui::PointerButton::Extra2 => 5,
    }
}

impl Detected {
    /// A key (with the modifier groups of a shortcut) the window saw.
    pub fn key(code: u32, groups: u8) -> Self {
        Detected { code, groups, ..Default::default() }
    }

    /// Mouse button `n` the window saw.
    pub fn button(n: u32) -> Self {
        let code = keynames::button_code(n).unwrap_or(keynames::BTN_LEFT);
        Detected { code, button: Some(n), is_button: true, ..Default::default() }
    }

    /// The trigger a rule for it names (`ctrl+alt+tab`, `f13`, `code:432`).
    pub fn trigger(&self) -> keynames::Trigger {
        keynames::Trigger { groups: self.groups, sided: 0, key: self.code as u16 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_read_back() {
        let found = r#"{"away":false,"button":null,"code":15,"device":{"fingerprint":"usb/046d:c52b//MX","name":"MX"},"kind":"key","name":"tab","result":"found","shortcut":true,"trigger":"ctrl+alt+tab","unusable":null}"#;
        let Outcome::Found(d) = parse(&format!("Press the key…\n{found}\n")) else { panic!() };
        assert_eq!((d.code, d.groups, d.is_button), (15, keynames::CTRL | keynames::ALT, false));
        assert_eq!(d.trigger().to_string(), "ctrl+alt+tab");
        assert_eq!(d.device, Some(("usb/046d:c52b//MX".into(), "MX".into())));
        let button = r#"{"button":7,"code":278,"device":null,"kind":"button","result":"found","trigger":"button7","unusable":null}"#;
        let Outcome::Found(d) = parse(button) else { panic!() };
        assert_eq!((d.button, d.is_button, d.device), (Some(7), true, None));
        assert_eq!(parse(r#"{"result":"timeout","seconds":60,"note":"x"}"#), Outcome::Timeout(60, Some("x".into())));
        assert_eq!(parse(r#"{"result":"replaced"}"#), Outcome::Cancelled);
        assert!(matches!(parse(r#"{"result":"not-running","message":"m"}"#), Outcome::Failed(m) if m.contains("isn't running")));
        assert!(matches!(parse(""), Outcome::Failed(_)));
        assert!(matches!(parse("{\"result\":\"found\""), Outcome::Failed(_)), "cut short");
        assert_eq!(Detected::button(6).code, 0x115);
    }
}
