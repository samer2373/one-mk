//! macOS: a notification that comes from the oneMK app (its name and icon, bundle id `com.one-mk.app`)
//! through `UNUserNotificationCenter`.
//!
//! The system only accepts that from a process that Launch Services started as the app (found on macOS
//! 15 and 26): one started by a shell or by launchd, like the daemon and the command line, is turned away
//! with "operation couldn't be completed" and no question to the person. So they start the app itself
//! with `--post-notification TITLE BODY` (`one-mk-app`, see [`post_args`]), which calls [`post`] and exits.

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotificationRequest,
    UNNotificationSettings, UNUserNotificationCenter,
};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::mpsc;
use std::time::Duration;

/// The first time, the system asks the person; this is how long to give them.
const ASK_WAIT: Duration = Duration::from_secs(90);
const POST_WAIT: Duration = Duration::from_secs(10);
/// `UNErrorCodeNotificationsNotAllowed`.
const NOT_ALLOWED: isize = 1;

/// What came of one notification.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Shown,
    /// Not shown, on purpose or because nobody answered the question; the words are in the log already.
    Quiet(&'static str),
    Failed(String),
}

/// What to do about one notification.
#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// Post it.
    Center,
    /// Ask the person first (the system shows its own question), then post.
    Ask,
    /// The person said no in System Settings: show nothing.
    Quiet,
    /// Not an app bundle: the center cannot be used.
    NoBundle,
}

pub fn route(bundle_id: Option<&str>, status: UNAuthorizationStatus) -> Route {
    if bundle_id.is_none_or(str::is_empty) {
        Route::NoBundle
    } else if status == UNAuthorizationStatus::Denied {
        Route::Quiet
    } else if status == UNAuthorizationStatus::NotDetermined {
        Route::Ask
    } else {
        Route::Center
    }
}

/// The app bundle around a program inside it (`one.app/Contents/MacOS/one-mk` -> `one.app`).
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let app = exe.parent()?.parent()?.parent()?;
    let named = app.extension().is_some_and(|e| e == "app");
    (named && exe.parent()?.file_name()? == "MacOS" && exe.parent()?.parent()?.file_name()? == "Contents")
        .then(|| app.to_path_buf())
}

/// The arguments for `open` that start `app` just to show this notification, without bringing it forward.
pub fn open_args(app: &Path, title: &str, body: &str) -> Vec<OsString> {
    let mut v: Vec<OsString> = ["-n", "-g", "-j", "-a"].map(Into::into).into();
    v.push(app.into());
    v.extend(["--args", "--post-notification", title, body].map(OsString::from));
    v
}

/// `--post-notification TITLE BODY` as given to the app.
pub fn post_args(args: &[OsString]) -> Option<(String, String)> {
    match args {
        [flag, title, body] if flag == "--post-notification" => Some((title.to_str()?.into(), body.to_str()?.into())),
        _ => None,
    }
}

/// Shows the notification, asking the person first if they have never been asked. Blocks until done
/// (or until the question times out): run it in a process of its own.
pub fn post(title: &str, body: &str) -> Outcome {
    let bundle = NSBundle::mainBundle().bundleIdentifier().map(|s| s.to_string());
    if route(bundle.as_deref(), UNAuthorizationStatus::Authorized) == Route::NoBundle {
        return Outcome::Failed("not running from an app bundle".into());
    }
    // Without a bundle `currentNotificationCenter` raises, so only past the check above.
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // SAFETY: the system hands over a live settings object for the length of the call.
        let _ = tx.send(unsafe { settings.as_ref() }.authorizationStatus());
    });
    center.getNotificationSettingsWithCompletionHandler(&block);
    let Ok(status) = rx.recv_timeout(POST_WAIT) else {
        return Outcome::Failed("no answer about the notification settings".into());
    };
    match route(bundle.as_deref(), status) {
        Route::NoBundle => return Outcome::Failed("not running from an app bundle".into()),
        Route::Quiet => return Outcome::Quiet("notifications are off for oneMK in System Settings"),
        Route::Ask => {
            let (tx, rx) = mpsc::channel();
            let block = RcBlock::new(move |granted: Bool, _: *mut NSError| {
                let _ = tx.send(granted.as_bool());
            });
            center.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert, &block);
            if !rx.recv_timeout(ASK_WAIT).unwrap_or(false) {
                return Outcome::Quiet("oneMK was not allowed to show notifications");
            }
        }
        Route::Center => {}
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    let id = format!("one-mk-{}", std::process::id());
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&id), &content, None);
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |error: *mut NSError| {
        // SAFETY: null, or an error that lives for the length of the call.
        let _ = tx.send(unsafe { error.as_ref() }.map(|e| (e.code(), e.localizedDescription().to_string())));
    });
    center.addNotificationRequest_withCompletionHandler(&request, Some(&block));
    match rx.recv_timeout(POST_WAIT) {
        Ok(None) => Outcome::Shown,
        Ok(Some((NOT_ALLOWED, _))) => Outcome::Quiet("the system says notifications are not allowed for oneMK"),
        Ok(Some((_, why))) => Outcome::Failed(why),
        Err(_) => Outcome::Failed("the notification center did not answer".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_by_bundle_and_permission() {
        use UNAuthorizationStatus as S;
        assert_eq!(route(None, S::Authorized), Route::NoBundle);
        assert_eq!(route(Some(""), S::Authorized), Route::NoBundle);
        assert_eq!(route(Some("com.one-mk.app"), S::Authorized), Route::Center);
        assert_eq!(route(Some("com.one-mk.app"), S::Provisional), Route::Center);
        assert_eq!(route(Some("com.one-mk.app"), S::NotDetermined), Route::Ask);
        assert_eq!(route(Some("com.one-mk.app"), S::Denied), Route::Quiet);
        assert_eq!(route(None, S::Denied), Route::NoBundle);
    }

    #[test]
    fn finds_the_bundle_around_a_program() {
        let app = Path::new("/Applications/one-mk.app");
        assert_eq!(bundle_of(&app.join("Contents/MacOS/one-mk")), Some(app.to_path_buf()));
        assert_eq!(bundle_of(Path::new("/tmp/target/debug/one-mk")), None);
        // Deep enough, but not a bundle layout.
        assert_eq!(bundle_of(Path::new("/x/y.app/bin/lib/one-mk")), None);
        assert_eq!(bundle_of(Path::new("one-mk")), None);
    }

    #[test]
    fn open_starts_the_app_with_the_words_after_args() {
        let v = open_args(Path::new("/A/one-mk.app"), "-t", "line\nbody");
        let v: Vec<_> = v.iter().map(|s| s.to_str().unwrap()).collect();
        assert_eq!(v, ["-n", "-g", "-j", "-a", "/A/one-mk.app", "--args", "--post-notification", "-t", "line\nbody"]);
    }

    #[test]
    fn post_notification_arguments_round_trip() {
        let args: Vec<OsString> = ["--post-notification", "T", "B"].map(Into::into).into();
        assert_eq!(post_args(&args), Some(("T".into(), "B".into())));
        assert_eq!(post_args(&args[..2]), None);
        assert_eq!(post_args(&["--home".into(), "x".into(), "y".into()]), None);
    }
}
