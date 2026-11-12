//! The clipboard between two real daemons, each with a private X server (Xvfb) and
//! `xclip` as the "application": text and images both ways, the switches and size
//! limit, fetching, the pointer taking the clipboard along, and no echo back. Skipped
//! (with a message) where Xvfb or xclip is not installed.
#![cfg(target_os = "linux")]

mod qa_common;
use qa_common::*;
use std::io::Write;
use std::process::{Command, Stdio};

fn display(node: &Node) -> String {
    node.env.iter().find(|(name, _)| name == "DISPLAY").unwrap().1.clone()
}

/// What a program would do on Ctrl+C: own the selection until someone else takes it.
#[allow(clippy::zombie_processes)] // xclip stays on as the selection owner on purpose
fn copy(node: &Node, target: &str, data: &[u8]) {
    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard", "-display", &display(node), "-t", target, "-i"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(data).unwrap();
    // xclip stays behind as the selection owner; it ends when another program takes the clipboard.
    std::thread::sleep(std::time::Duration::from_millis(150));
}

fn paste(node: &Node, target: &str) -> Option<Vec<u8>> {
    let out = Command::new("timeout")
        .args(["4", "xclip", "-selection", "clipboard", "-display", &display(node), "-t", target, "-o"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
}

fn paste_text(node: &Node) -> Option<String> {
    paste(node, "UTF8_STRING").and_then(|bytes| String::from_utf8(bytes).ok())
}

fn png(seed: u8) -> Vec<u8> {
    let mut image = image::RgbaImage::new(48, 32);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = image::Rgba([(x as u8).wrapping_mul(5).wrapping_add(seed), (y as u8).wrapping_mul(7), seed, 255]);
    }
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn pixels(png: &[u8]) -> Vec<u8> {
    image::load_from_memory(png).unwrap().to_rgba8().into_raw()
}

fn setup() -> Option<(Lab, Node, Node)> {
    let mut lab = Lab::new();
    let (mut alpha, mut beta) = (lab.node("alpha"), lab.node("beta"));
    if !lab.display(&mut alpha) || !lab.display(&mut beta) {
        eprintln!("SKIP clipboard e2e: Xvfb and xclip are not both installed");
        return None;
    }
    lab.start(&alpha, &[]);
    lab.start(&beta, &[]);
    lab.pair(&alpha, &beta);
    Some((lab, alpha, beta))
}

#[test]
fn clip_send_and_clip_get_move_text_and_images_and_every_refusal_says_why() {
    if !isolated("clip_send_and_clip_get_move_text_and_images_and_every_refusal_says_why") {
        return;
    }
    let Some((lab, alpha, beta)) = setup() else { return };

    // Text, including what breaks naive code: newlines, tabs, quotes, emoji, a lone surrogate-free CJK run.
    let text = "line one\nline\ttwo \"quoted\" 'single' $HOME `x` \\ ünï 日本語 🎉\n\nlast";
    copy(&alpha, "UTF8_STRING", text.as_bytes());
    let said = Lab::ok(&alpha, &["clip-send", "--to", "beta"]);
    assert!(said.contains("Sent your clipboard") && said.contains("beta"), "{said}");
    wait("beta clipboard to hold the text", 15, || (paste_text(&beta).as_deref() == Some(text)).then_some(()));

    // The other way, with the only connected computer implied.
    copy(&beta, "UTF8_STRING", "from beta".as_bytes());
    assert!(Lab::ok(&beta, &["clip-send"]).contains("Sent your clipboard"));
    wait("alpha clipboard to hold beta's text", 15, || {
        (paste_text(&alpha).as_deref() == Some("from beta")).then_some(())
    });

    // An image comes out the same picture, whatever the encoder did on the way.
    let picture = png(3);
    copy(&alpha, "image/png", &picture);
    let said = Lab::ok(&alpha, &["clip-send", "--to", "beta"]);
    assert!(said.contains("image"), "{said}");
    let got = wait("beta to hold the image", 15, || paste(&beta, "image/png"));
    assert_eq!(pixels(&got), pixels(&picture), "the picture changed on the way");

    // Nothing to send: said plainly (an empty clipboard is not an error that kills anything).
    // A kind that is switched off on the receiver is refused with the reason, and the receiver is untouched.
    Lab::ok(&beta, &["clipboard", "--image", "off"]);
    copy(&alpha, "image/png", &png(9));
    let refused = Lab::run(&alpha, &["clip-send", "--to", "beta"]);
    assert!(refused.failed().contains("switched off"), "{refused:?}");
    assert_eq!(pixels(&paste(&beta, "image/png").unwrap()), pixels(&picture), "a refused image still reached beta");
    Lab::ok(&beta, &["clipboard", "--image", "on", "--max-mb", "1"]);

    // Over the receiver's limit.
    let big = "x".repeat(2 << 20);
    copy(&alpha, "UTF8_STRING", big.as_bytes());
    let too_big = Lab::run(&alpha, &["clip-send", "--to", "beta"]);
    assert!(too_big.failed().contains("size limit"), "{too_big:?}");
    Lab::ok(&beta, &["clipboard", "--max-mb", "25"]);

    // Fetching is off until the other computer allows it.
    copy(&beta, "UTF8_STRING", "secret-ish text on beta".as_bytes());
    let denied = Lab::run(&alpha, &["clip-get", "--from", "beta"]);
    assert!(denied.failed().contains("allow-fetch") && denied.stderr.contains("beta"), "{denied:?}");
    Lab::ok(&beta, &["clipboard", "--allow-fetch", "on"]);
    let got = Lab::ok(&alpha, &["clip-get", "--from", "beta"]);
    assert!(got.contains("Copied beta's clipboard"), "{got}");
    wait("alpha to hold beta's text", 15, || {
        (paste_text(&alpha).as_deref() == Some("secret-ish text on beta")).then_some(())
    });

    // Unknown computers, and a daemon with the clipboard off.
    assert!(Lab::run(&alpha, &["clip-send", "--to", "nobody"]).failed().contains("nobody"));
    assert!(Lab::run(&alpha, &["clip-get", "--from", "nobody"]).failed().contains("nobody"));
    let mut lab = lab;
    let gamma = lab.node("gamma");
    lab.start(&gamma, &["--no-clipboard"]);
    let out = Lab::run(&gamma, &["clip-send", "--to", "alpha"]);
    assert!(out.failed().contains("clipboard is not shared"), "{out:?}");
    // The daemons are all still fine.
    for node in [&alpha, &beta, &gamma] {
        assert!(lab.status_of(node).is_some());
        let log = Lab::log(node);
        assert!(!log.contains("panicked"), "{log}");
    }
}

/// The pointer taking the clipboard along, and no echo: a clipboard that arrives is not sent
/// back, so two computers never ping-pong the same copy.
#[test]
fn the_clipboard_follows_the_pointer_without_echoing_back() {
    if !isolated("the_clipboard_follows_the_pointer_without_echoing_back") {
        return;
    }
    let Some((lab, alpha, beta)) = setup() else { return };
    for node in [&alpha, &beta] {
        Lab::ok(node, &["clipboard", "history", "on"]);
    }
    let history = |node: &Node| -> Vec<serde_json::Value> {
        let out = Lab::run(node, &["clipboard", "history", "list", "--json"]);
        let json: serde_json::Value = serde_json::from_str(&out.stdout)
            .unwrap_or_else(|e| panic!("history list --json is not JSON ({e}): {out:?}"));
        json["items"].as_array().cloned().unwrap_or_else(|| json.as_array().cloned().unwrap_or_default())
    };
    copy(&alpha, "UTF8_STRING", b"carried along");
    wait("alpha's own copy in its history", 15, || (history(&alpha).len() == 1).then_some(()));
    Lab::ok(&alpha, &["switch", "beta"]);
    wait("beta to hold the carried text", 15, || (paste_text(&beta).as_deref() == Some("carried along")).then_some(()));
    // Nothing comes back: neither a second item on alpha nor a changed clipboard, however long we look.
    std::thread::sleep(std::time::Duration::from_secs(3));
    assert_eq!(history(&alpha).len(), 1, "the clipboard echoed back to where it came from: {:?}", history(&alpha));
    assert_eq!(history(&beta).len(), 1, "beta kept more than the one copy it was given: {:?}", history(&beta));
    assert_eq!(paste_text(&alpha).as_deref(), Some("carried along"));
    // Control back and forth does not multiply it either.
    Lab::ok(&alpha, &["switch", "here"]);
    Lab::ok(&alpha, &["switch", "beta"]);
    Lab::ok(&alpha, &["switch", "here"]);
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert_eq!(history(&alpha).len(), 1, "{:?}", history(&alpha));
    assert_eq!(history(&beta).len(), 1, "{:?}", history(&beta));

    // History through the command line: list, show, copy, delete, clear, with JSON that parses.
    let items = history(&beta);
    let number = items[0]["id"].as_u64().or_else(|| items[0]["number"].as_u64()).expect("an item number");
    let number = number.to_string();
    let shown = Lab::run(&beta, &["clipboard", "history", "show", &number, "--json"]);
    let shown: serde_json::Value = serde_json::from_str(&shown.success()).unwrap();
    assert!(shown.to_string().contains("carried along"), "{shown}");
    // An item number that is not there is refused with a message.
    Lab::run(&beta, &["clipboard", "history", "show", "99999"]).failed();
    copy(&beta, "UTF8_STRING", b"something else");
    wait("a second item", 15, || (history(&beta).len() == 2).then_some(()));
    Lab::ok(&beta, &["clipboard", "history", "copy", &number]);
    wait("the old text back on beta's clipboard", 15, || {
        (paste_text(&beta).as_deref() == Some("carried along")).then_some(())
    });
    Lab::ok(&beta, &["clipboard", "history", "delete", &number]);
    assert_eq!(history(&beta).len(), 1);
    Lab::ok(&beta, &["clipboard", "history", "clear"]);
    assert!(history(&beta).is_empty());
    Lab::ok(&beta, &["clipboard", "history", "off"]);
    assert!(lab.status_of(&beta).is_some());
}
