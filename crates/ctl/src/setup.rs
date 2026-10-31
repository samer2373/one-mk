//! First-run setup: what opening the window for the first time does so that
//! nobody has to type `one-mk install`.
//!
//! The window calls [`run`] once per settings folder. It starts one-mk at
//! login, adds the file manager entries (Linux) and starts the background
//! service, then leaves a marker file. The marker is why it never runs
//! again: someone who later turns "Start at login" off keeps it off.
//!
//! Everything goes through the [`Host`] trait so the order and the failure
//! handling are tested without touching the real login items.

use std::path::Path;

/// Present in the settings folder once first-run setup has run.
pub const MARKER: &str = "setup-done";

/// How one step of setup went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Done now.
    Done,
    /// Was already the way it should be; nothing was changed.
    Already,
    /// Not applicable here (no supported file manager, no companion program).
    Skipped(String),
    /// Failed; the message says what to do. Setup went on with the next step.
    Failed(String),
}

impl Step {
    pub fn failed(&self) -> Option<&str> {
        match self {
            Step::Failed(e) => Some(e),
            _ => None,
        }
    }
}

/// What first-run setup did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// one-mk starts when you log in.
    pub login: Step,
    /// "Send to" entries in the file manager.
    pub integrations: Step,
    /// The background service runs now.
    pub service: Step,
}

impl Report {
    /// Every failure, one sentence each, for the window to show.
    pub fn problems(&self) -> Vec<String> {
        [("Start at login", &self.login), ("File manager menu", &self.integrations), ("Starting one·mk", &self.service)]
            .into_iter()
            .filter_map(|(what, step)| step.failed().map(|e| format!("{what}: {e}")))
            .collect()
    }

    /// Something was changed (so the window may say "all set").
    pub fn changed(&self) -> bool {
        [&self.login, &self.integrations, &self.service].iter().any(|s| **s == Step::Done)
    }
}

/// What setup needs from the system.
pub trait Host {
    /// one-mk is set to start at login.
    fn login_installed(&self) -> bool;
    /// Starts one-mk at login (and now, if it is not running).
    fn install_login(&self) -> Result<(), String>;
    /// File manager entries exist already.
    fn integrations_present(&self) -> bool;
    /// Adds them; `Ok(false)`: there is no supported file manager.
    fn install_integrations(&self) -> Result<bool, String>;
    /// The service answers on its control channel.
    fn running(&self) -> bool;
    /// Starts the service for this session.
    fn start(&self) -> Result<(), String>;
}

/// True until [`run`] has run once in `dir`.
pub fn needed(dir: &Path) -> bool {
    !dir.join(MARKER).exists()
}

/// Runs first-run setup if it has not run in `dir`, `None` if it had.
///
/// A settings folder that already has login start turned on (someone ran
/// `one-mk install`) counts as set up. Every step is tried even when one
/// fails, and the marker is written either way: a failure is shown once and
/// the switches in the window stay available, rather than the same error
/// coming back at every launch.
pub fn run(dir: &Path, host: &dyn Host) -> Option<Report> {
    if !needed(dir) {
        return None;
    }
    let login = if host.login_installed() {
        Step::Already
    } else {
        host.install_login().map_or_else(Step::Failed, |()| Step::Done)
    };
    let integrations = if host.integrations_present() {
        Step::Already
    } else {
        match host.install_integrations() {
            Ok(true) => Step::Done,
            Ok(false) => Step::Skipped("no supported file manager".into()),
            Err(e) => Step::Failed(e),
        }
    };
    // `install_login` starts the service itself when it is not running; this
    // covers a login entry that was already there, or one that could not start it.
    let service = if host.running() {
        if login == Step::Done { Step::Done } else { Step::Already }
    } else {
        host.start().map_or_else(Step::Failed, |()| Step::Done)
    };
    // Writing the marker is best effort: if the folder is read-only, setup
    // runs again next time, which is harmless because every step is idempotent.
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join(MARKER), b"1\n");
    Some(Report { login, integrations, service })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    struct Fake {
        login: Cell<bool>,
        integrations: Cell<bool>,
        running: Cell<bool>,
        fail_login: Cell<bool>,
        fail_start: Cell<bool>,
        no_file_manager: Cell<bool>,
        calls: RefCell<Vec<&'static str>>,
    }

    impl Host for Fake {
        fn login_installed(&self) -> bool {
            self.login.get()
        }
        fn install_login(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("login");
            if self.fail_login.get() {
                return Err("no home directory".into());
            }
            self.login.set(true);
            self.running.set(true); // like the real one: it starts the service too
            Ok(())
        }
        fn integrations_present(&self) -> bool {
            self.integrations.get()
        }
        fn install_integrations(&self) -> Result<bool, String> {
            self.calls.borrow_mut().push("integrations");
            if self.no_file_manager.get() {
                return Ok(false);
            }
            self.integrations.set(true);
            Ok(true)
        }
        fn running(&self) -> bool {
            self.running.get()
        }
        fn start(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("start");
            if self.fail_start.get() {
                return Err("one-mk didn't start".into());
            }
            self.running.set(true);
            Ok(())
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("one-mk-setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_fresh_folder_gets_everything_once() {
        let dir = scratch("fresh");
        let host = Fake::default();
        let report = run(&dir, &host).expect("first run");
        assert_eq!(report, Report { login: Step::Done, integrations: Step::Done, service: Step::Done });
        assert!(report.changed() && report.problems().is_empty());
        assert_eq!(*host.calls.borrow(), ["login", "integrations"], "login start already started the service");
        assert!(!needed(&dir));
        assert_eq!(run(&dir, &host), None, "never twice");
        assert_eq!(host.calls.borrow().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn someone_who_ran_install_by_hand_is_left_alone() {
        let dir = scratch("by-hand");
        let host = Fake::default();
        host.login.set(true);
        host.integrations.set(true);
        host.running.set(true);
        let report = run(&dir, &host).unwrap();
        assert_eq!(report, Report { login: Step::Already, integrations: Step::Already, service: Step::Already });
        assert!(!report.changed());
        assert!(host.calls.borrow().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_login_entry_without_a_running_service_starts_it() {
        let dir = scratch("not-running");
        let host = Fake::default();
        host.login.set(true);
        let report = run(&dir, &host).unwrap();
        assert_eq!(report.login, Step::Already);
        assert_eq!(report.service, Step::Done);
        assert!(host.calls.borrow().contains(&"start"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failure_does_not_stop_the_other_steps_and_is_reported_once() {
        let dir = scratch("fail");
        let host = Fake::default();
        host.fail_login.set(true);
        host.fail_start.set(true);
        host.no_file_manager.set(true);
        let report = run(&dir, &host).unwrap();
        assert_eq!(report.login, Step::Failed("no home directory".into()));
        assert_eq!(report.integrations, Step::Skipped("no supported file manager".into()));
        assert_eq!(report.service, Step::Failed("one-mk didn't start".into()));
        assert_eq!(
            report.problems(),
            ["Start at login: no home directory", "Starting one·mk: one-mk didn't start"]
        );
        assert_eq!(run(&dir, &host), None, "the error is not repeated at every launch");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unwritable_folder_only_means_setup_may_run_again() {
        let dir = scratch("readonly");
        let blocker = dir.with_extension("file");
        std::fs::write(&blocker, b"x").unwrap();
        let host = Fake::default();
        // A folder that cannot be made: the marker can't be written.
        assert!(run(&blocker.join("inside"), &host).is_some());
        assert!(needed(&blocker.join("inside")));
        let _ = std::fs::remove_file(&blocker);
    }
}
