//! Pairing from the window: runs `one-mk pair --machine` and turns its
//! event lines into [`Step`]s; the person's answer goes back on its stdin.
//! The code shown is whatever `one-mk` computed; this file only relays it.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Waiting for the other computer to start pairing too.
    Searching { me: String },
    /// Nothing found for a while: show what to check.
    Hint,
    /// A computer answered; the code exchange is running.
    Found,
    /// Compare this code with the other screen.
    Code { name: String, code: String },
    /// We said yes; waiting for the other side.
    Waiting { name: String },
    Paired { id: String, name: String },
    /// Over without pairing: why.
    Ended(String),
    /// The attempt failed but pairing goes on waiting.
    Retry(String),
}

pub struct Pairing {
    child: Child,
    stdin: Option<ChildStdin>,
    pub rx: Receiver<Step>,
}

/// Parses one event line from `one-mk pair --machine`.
pub fn parse(line: &str) -> Option<Step> {
    let f: Vec<&str> = line.trim_end_matches(['\r', '\n']).split('\t').collect();
    Some(match f[..] {
        ["me", name, ..] => Step::Searching { me: name.into() },
        ["searching"] => return None,
        ["hint"] => Step::Hint,
        ["peer", ..] => Step::Found,
        ["code", name, code] => Step::Code { name: name.into(), code: code.chars().filter(char::is_ascii_digit).collect() },
        ["waiting"] => Step::Waiting { name: String::new() },
        ["paired", id, name, _side] => Step::Paired { id: short(id), name: name.into() },
        ["cancelled"] => Step::Ended("Pairing cancelled. Nothing was saved.".into()),
        ["declined"] => Step::Ended("The other computer did not confirm. Nothing was saved.".into()),
        ["retry", why] => Step::Retry(why.into()),
        _ => return None,
    })
}

/// The short id (`abcd-ef01`) of a full fingerprint, as status reports it.
fn short(fp: &str) -> String {
    if fp.len() >= 8 && !fp.contains('-') { format!("{}-{}", &fp[..4], &fp[4..8]) } else { fp.into() }
}

impl Pairing {
    pub fn start(daemon: &std::path::Path, home_args: &[String], ctx: egui::Context) -> std::io::Result<Self> {
        let mut cmd = Command::new(daemon);
        cmd.args(["pair", "--machine"]).args(home_args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = one_mk_ctl::proc::hide_window(&mut cmd).spawn()?;
        let (tx, rx) = channel();
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let (tx2, ctx2) = (tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let mut err = String::new();
            // Log lines go here too; the final error is the one-line `one-mk: ...`.
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if line.starts_with("one-mk: ") {
                    err = line;
                }
            }
            if !err.is_empty() {
                let _ = tx2.send(Step::Ended(err.trim_start_matches("one-mk: ").to_owned()));
                ctx2.request_repaint();
            }
        });
        std::thread::spawn(move || {
            let mut last_name = String::new();
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Some(mut step) = parse(&line) else { continue };
                match &mut step {
                    Step::Code { name, .. } => last_name = name.clone(),
                    Step::Waiting { name } => *name = last_name.clone(),
                    _ => {}
                }
                if tx.send(step).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Ok(Self { stdin: child.stdin.take(), child, rx })
    }

    pub fn answer(&mut self, yes: bool) {
        if let Some(s) = &mut self.stdin {
            let _ = s.write_all(if yes { b"yes\n" } else { b"no\n" });
            let _ = s.flush();
        }
    }
}

impl Drop for Pairing {
    /// Closing the sheet stops pairing.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_events() {
        assert_eq!(parse("me\tdesk\tabcd-ef01\t4000\n"), Some(Step::Searching { me: "desk".into() }));
        assert_eq!(parse("searching"), None);
        assert_eq!(parse("code\tStudio PC\t482 913"), Some(Step::Code { name: "Studio PC".into(), code: "482913".into() }));
        assert_eq!(
            parse("paired\t0123456789abcdef\tStudio PC\tright"),
            Some(Step::Paired { id: "0123-4567".into(), name: "Studio PC".into() })
        );
        assert!(matches!(parse("declined"), Some(Step::Ended(_))));
        assert_eq!(parse("retry\ttimed out"), Some(Step::Retry("timed out".into())));
        assert_eq!(parse("nonsense\tline"), None);
    }
}
