use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use one_mk_net::{Fingerprint, TrustStore};
use one_mk_proto::wallpaper::{Kind, Message};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[path = "wallpaper_extension.rs"]
mod extension;

#[derive(Default)]
pub struct Ingress { window: Option<(std::time::Instant,u8)> }
impl Ingress {
    pub fn admit(&mut self) -> bool {
        let now = std::time::Instant::now();
        let (start,count) = self.window.get_or_insert((now,0));
        if now.duration_since(*start) >= std::time::Duration::from_secs(1) { *start = now; *count = 0; }
        if *count == 64 { return false; }
        *count += 1;
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PolicyFile")]
pub struct WallpaperSettings {
    pub version: u16,
    pub enabled: bool,
    pub grants: BTreeMap<String,u64>,
}

/// Reads `[wallpaper]`; a section that cannot be read (a newer version's, a
/// typo, a hand edit) means the feature is off with no consent, and never
/// costs the rest of `settings.toml`.
pub fn lenient<'de, D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<WallpaperSettings, D::Error> {
    let value = toml::Value::deserialize(deserializer)?;
    Ok(WallpaperSettings::deserialize(value).unwrap_or_else(|error| {
        tracing::warn!("[wallpaper] in settings.toml is ignored (wallpaper stays off, no consent granted): {error}");
        WallpaperSettings::default()
    }))
}

impl Default for WallpaperSettings {
    fn default() -> Self { Self { version: 1, enabled: false, grants: BTreeMap::new() } }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PolicyFile { version: u16, enabled: bool, grants: BTreeMap<String,u64> }
impl Default for PolicyFile {
    fn default() -> Self { Self { version: 1, enabled: false, grants: BTreeMap::new() } }
}
impl TryFrom<PolicyFile> for WallpaperSettings {
    type Error = &'static str;
    fn try_from(file: PolicyFile) -> std::result::Result<Self,Self::Error> {
        if file.version != 1 || file.grants.len() > 8 || file.grants.iter().any(|(peer,nonce)| {
            *nonce == 0 || peer.len() != 64 || !peer.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }) { return Err("wallpaper settings need version 1 and at most 8 canonical fingerprints with nonzero consent revisions"); }
        Ok(Self { version: file.version, enabled: file.enabled, grants: file.grants })
    }
}

struct Receiver {
    consent: u64,
    epoch: [u8;16],
    sequence: u64,
    window_ms: u64,
    budget: u8,
    announced: bool,
    revoked: bool,
    canvas: Option<[u8;48]>,
}

impl Receiver {
    fn new(consent: u64) -> Self {
        let mut epoch = rand::random();
        while epoch == [0;16] { epoch = rand::random(); }
        Self { consent,epoch,sequence: 0,window_ms: 0,budget: 16,announced: false,revoked: false,canvas: None }
    }
}

#[derive(Default)]
pub struct Runtime {
    origin: Option<std::time::Instant>,
    receivers: BTreeMap<Fingerprint,Receiver>,
    accepted: u64,
    rejected: u64,
    rejection_reasons: BTreeMap<&'static str,u64>,
}

impl Runtime {
    pub fn refresh(&mut self, settings: &WallpaperSettings) {
        if settings.enabled { self.origin.get_or_insert_with(std::time::Instant::now); }
        else { self.origin = None; }
        self.receivers.retain(|peer,receiver| settings.enabled && settings.grants.get(&peer.to_string()) == Some(&receiver.consent));
        if settings.enabled {
            for (peer,consent) in &settings.grants {
                if let Ok(peer) = peer.parse() { self.receivers.entry(peer).or_insert_with(|| Receiver::new(*consent)); }
            }
        }
    }

    pub fn epoch(&self, peer: Fingerprint) -> Option<[u8;16]> {
        self.receivers.get(&peer).filter(|receiver| !receiver.revoked).map(|receiver| receiver.epoch)
    }

    pub fn disconnected(&mut self, peer: Fingerprint) {
        if let Some(receiver) = self.receivers.get_mut(&peer) && !receiver.revoked {
            *receiver = Receiver::new(receiver.consent);
        }
    }

    pub fn announcements(&mut self, peer: Fingerprint) -> Vec<Message> {
        let Some(epoch) = self.epoch(peer) else { return Vec::new() };
        let receiver = self.receivers.get_mut(&peer).unwrap();
        if receiver.announced { return Vec::new(); }
        receiver.announced = true;
        [Kind::Version,Kind::Renderer,Kind::ConsentGrant].into_iter().enumerate().map(|(index,kind)| Message {
            kind,coordinator: peer.0,epoch,sequence: index as u64+1,descriptor: [0;48],
        }).collect()
    }

    pub fn receive(&mut self, peer: Fingerprint, message: Message, now_ms: u64) -> std::result::Result<(),&'static str> {
        let result = self.admit(peer,message,now_ms);
        if result.is_ok() { self.accepted = self.accepted.saturating_add(1); }
        else if let Err(reason) = result {
            self.rejected = self.rejected.saturating_add(1);
            let count = self.rejection_reasons.entry(reason).or_default();
            *count = count.saturating_add(1);
        }
        result
    }

    pub fn receive_now(&mut self, peer: Fingerprint, message: Message) {
        let now = self.origin.map_or(0,|origin| origin.elapsed().as_millis().min(u128::from(u64::MAX)) as u64);
        let _ = self.receive(peer,message,now);
    }

    fn admit(&mut self, peer: Fingerprint, message: Message, now_ms: u64) -> std::result::Result<(),&'static str> {
        let Some(receiver) = self.receivers.get_mut(&peer).filter(|receiver| !receiver.revoked) else { return Err("wallpaper is off or this receiver has not consented"); };
        if now_ms.saturating_sub(receiver.window_ms) >= 1000 {
            receiver.window_ms = now_ms; receiver.budget = 16;
        }
        if receiver.budget == 0 { return Err("wallpaper rate limit"); }
        receiver.budget -= 1;
        if message.coordinator != peer.0 { return Err("wrong receiver identity"); }
        if message.epoch != receiver.epoch { return Err("wrong receiver epoch"); }
        if message.sequence <= receiver.sequence { return Err("replayed or stale sequence"); }
        if message.kind == Kind::ConsentGrant { return Err("only the receiving owner can grant consent"); }
        receiver.sequence = message.sequence;
        match message.kind {
            Kind::ConsentRevoke => { receiver.revoked = true; receiver.canvas = None; }
            Kind::Canvas => receiver.canvas = Some(message.descriptor),
            _ => {}
        }
        Ok(())
    }

    pub fn status(&self) -> String {
        serde_json::json!({"accepted": self.accepted,"rejected": self.rejected,
            "rejection_reasons": self.rejection_reasons,
            "receivers": self.receivers.values().filter(|receiver| !receiver.revoked).count(),
            "pending_canvases": self.receivers.values().filter(|receiver| receiver.canvas.is_some()).count(),
            "renderer": "unavailable"}).to_string()
    }
}

#[derive(Args)]
pub struct WallpaperArgs {
    /// Print machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: WallpaperCommand,
}

#[derive(Subcommand)]
enum WallpaperCommand {
    /// Manage the optional GNOME extension (static scaffolding, not playback).
    Extension { #[command(subcommand)] cmd: extension::Command },
    /// Show wallpaper policy and renderer availability.
    Status,
    /// Enable experimental wallpaper policy; does not start playback.
    Enable,
    /// Disable wallpaper policy without deleting saved consent.
    Disable,
    /// Check wallpaper prerequisites and explain unavailable features.
    Doctor,
    /// Manage which paired computers may coordinate this receiver.
    Consent { #[command(subcommand)] cmd: ConsentCommand },
    /// Import a video or animated image through the separately installed,
    /// sandboxed importer (VP9/AV1 WebM, GIF, APNG, animated WebP). Run
    /// `one-mk wallpaper import --help` for its options.
    Import {
        #[arg(num_args = 0.., allow_hyphen_values = true)]
        arguments: Vec<std::ffi::OsString>,
    },
}

#[derive(Subcommand)]
enum ConsentCommand {
    /// Allow a directly paired computer to coordinate this receiver.
    Grant {
        /// Paired computer's name or full fingerprint.
        peer: String,
    },
    /// Withdraw a paired computer's permission on this receiver.
    Revoke {
        /// Paired computer's name or full fingerprint.
        peer: String,
    },
}

#[derive(Debug)]
pub struct Failure { pub code: u8, message: String }
impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { formatter.write_str(&self.message) }
}
impl std::error::Error for Failure {}

pub fn summary(settings: &WallpaperSettings) -> String {
    format!("Wallpaper: {}; {} receiver-approved coordinator{}; renderer unavailable; no playback or timers.",
        if settings.enabled { "enabled (scaffolding only)" } else { "off" }, settings.grants.len(), if settings.grants.len() == 1 { "" } else { "s" })
}

pub fn forget(dir: &Path, fingerprint: &str) -> Result<()> {
    if crate::settings::Settings::load(dir)?.wallpaper.grants.contains_key(fingerprint) {
        crate::settings::Settings::update(dir,|settings| { settings.wallpaper.grants.remove(fingerprint); Ok(()) })?;
    }
    Ok(())
}

pub async fn command(dir: &Path, args: WallpaperArgs) -> Result<()> {
    let result = execute(dir,&args.cmd).await;
    match result {
        Ok(value) => {
            if args.json { println!("{value}"); }
            else { println!("{}",value["message"].as_str().unwrap_or("Wallpaper settings saved.")); }
            Ok(())
        }
        Err(error) => {
            let code = error.downcast_ref::<Failure>().map_or(5,|failure| failure.code);
            if args.json { println!("{}",serde_json::json!({"ok": false,"code": code,"error": error.to_string()})); }
            Err(Failure { code,message: error.to_string() }.into())
        }
    }
}

async fn execute(dir: &Path, command: &WallpaperCommand) -> Result<serde_json::Value> {
    use crate::settings::Settings;
    if let WallpaperCommand::Extension { cmd } = command { return extension::command(cmd).await; }
    let mut settings = Settings::load(dir)?;
    match command {
        WallpaperCommand::Extension { .. } => unreachable!(),
        WallpaperCommand::Status => {}
        WallpaperCommand::Doctor => {
            let state = extension::status().await?;
            let message = format!("{}\n{}", summary(&settings.wallpaper), state["message"].as_str().unwrap_or("extension state unknown"));
            if settings.wallpaper.enabled { return Err(Failure { code: 3, message }.into()); }
            return Ok(serde_json::json!({"ok": true, "message": message, "extension": state}));
        }
        // Dispatched to the importer before this code runs.
        // Only reached when something stands between `wallpaper` and `import`
        // (a global option such as -v, --home or --json).
        WallpaperCommand::Import { .. } => return Err(Failure { code: 2, message: "write `one-mk wallpaper import FILE ...` with `import` right after `wallpaper`, and options after it".into() }.into()),
        WallpaperCommand::Enable | WallpaperCommand::Disable => {
            settings = Settings::update(dir,|settings| { settings.wallpaper.enabled = matches!(command,WallpaperCommand::Enable); Ok(settings.clone()) })?;
            apply(dir).await?;
        }
        WallpaperCommand::Consent { cmd } => {
            let (query,grant) = match cmd { ConsentCommand::Grant { peer } => (peer,true),ConsentCommand::Revoke { peer } => (peer,false) };
            let trust = TrustStore::load(dir)?;
            let fingerprint = if grant {
                let Some(device) = trust.find(query) else { return Err(Failure { code: 4,message: "Consent requires one directly paired computer; use its full fingerprint if the name is ambiguous.".into() }.into()) };
                device.fingerprint.to_ascii_lowercase()
            } else if query.len() == 64 && query.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
                query.clone()
            } else {
                let Some(device) = trust.find(query) else { return Err(Failure { code: 4,message: "Computer not found. Revoke using its full saved fingerprint, even after forgetting it.".into() }.into()) };
                device.fingerprint.to_ascii_lowercase()
            };
            settings = Settings::update(dir,|settings| {
                if grant {
                    if settings.wallpaper.grants.len() >= 8 && !settings.wallpaper.grants.contains_key(&fingerprint) {
                        return Err(Failure { code: 2,message: "At most eight wallpaper coordinators can have consent. Revoke one first.".into() }.into());
                    }
                    settings.wallpaper.grants.insert(fingerprint.clone(),// TOML integers are signed 64-bit: a larger revision could not be saved.
                    (rand::random::<u64>() >> 1).max(1));
                } else { settings.wallpaper.grants.remove(&fingerprint); }
                Ok(settings.clone())
            })?;
            apply(dir).await?;
        }
    }
    let runtime = crate::control::request(dir,crate::control::Request::WallpaperStatus).await?;
    let runtime = runtime.map(|text| serde_json::from_str::<serde_json::Value>(&text)).transpose()?;
    Ok(serde_json::json!({"ok": true,"version": settings.wallpaper.version,"enabled": settings.wallpaper.enabled,
        "consent": settings.wallpaper.grants.keys().collect::<Vec<_>>(),"renderer": "unavailable","playing": false,
        "message": summary(&settings.wallpaper),"daemon": runtime}))
}

async fn apply(dir: &Path) -> Result<()> {
    if let Some(reply) = crate::control::request(dir,crate::control::Request::Settings).await? && reply != "ok\n" {
        bail!("Wallpaper policy saved but the daemon did not apply it: {}. Stop this daemon before continuing.",reply.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn status_does_not_invent_resource_measurements() {
        let status: serde_json::Value = serde_json::from_str(&super::Runtime::default().status()).unwrap();
        assert!(status.get("timers").is_none());
        assert!(status.get("children").is_none());
    }
    use super::*;

    fn message(epoch: [u8;16], sequence: u64) -> Message {
        Message { kind: Kind::Renderer, coordinator: [7;32], epoch, sequence, descriptor: [0;48] }
    }

    #[test]
    fn hostile_peer_cannot_self_grant_replay_spoof_or_keep_authority_after_revoke() {
        let peer = Fingerprint([7;32]);
        let mut runtime = Runtime::default();
        let mut settings = WallpaperSettings::default();
        assert!(runtime.receive(peer,message([9;16],1),0).is_err());
        settings.enabled = true;
        runtime.refresh(&settings);
        assert!(runtime.receive(peer,message([9;16],1),0).is_err());
        settings.grants.insert(peer.to_string(),1);
        runtime.refresh(&settings);
        let epoch = runtime.epoch(peer).unwrap();
        assert!(runtime.receive(peer,message(epoch,1),0).is_ok());
        assert_eq!(runtime.receive(peer,message(epoch,1),0),Err("replayed or stale sequence"));
        assert_eq!(runtime.receive(peer,message([9;16],2),0),Err("wrong receiver epoch"));
        let mut spoof = message(epoch,2); spoof.coordinator = [8;32];
        assert!(runtime.receive(peer,spoof,0).is_err());
        let mut self_grant = message(epoch,2); self_grant.kind = Kind::ConsentGrant;
        assert_eq!(runtime.receive(peer,self_grant,0),Err("only the receiving owner can grant consent"));
        settings.grants.clear(); runtime.refresh(&settings);
        assert!(runtime.epoch(peer).is_none());
        assert!(runtime.receive(peer,message(epoch,3),1).is_err());
        settings.grants.insert(peer.to_string(),2); runtime.refresh(&settings);
        assert_ne!(runtime.epoch(peer).unwrap(),epoch);
        settings.enabled = false; runtime.refresh(&settings);
        assert!(runtime.receivers.is_empty());
    }

    #[test]
    fn flooding_and_remote_revoke_fail_closed_without_timers() {
        let peer = Fingerprint([7;32]);
        let mut settings = WallpaperSettings { enabled: true, ..Default::default() };
        settings.grants.insert(peer.to_string(),1);
        let mut runtime = Runtime::default(); runtime.refresh(&settings);
        let epoch = runtime.epoch(peer).unwrap();
        for sequence in 1..=16 { assert!(runtime.receive(peer,message(epoch,sequence),0).is_ok()); }
        assert_eq!(runtime.receive(peer,message(epoch,17),0),Err("wallpaper rate limit"));
        let mut revoke = message(epoch,18); revoke.kind = Kind::ConsentRevoke;
        assert!(runtime.receive(peer,revoke,1000).is_ok());
        assert!(runtime.receive(peer,message(epoch,19),1000).is_err());
        runtime.refresh(&settings);
        assert!(runtime.epoch(peer).is_none());
    }

    #[test]
    fn old_settings_migrate_to_off_with_no_consent() {
        let settings: crate::settings::Settings = toml::from_str("notify_files = false\n").unwrap();
        assert_eq!(settings.wallpaper,WallpaperSettings::default());
        assert!(!settings.wallpaper.enabled);
        // The policy itself stays strict; the section failing leaves the feature off.
        assert!(toml::from_str::<WallpaperSettings>("version=2\n").is_err());
        assert!(toml::from_str::<WallpaperSettings>("[grants]\nevil=1\n").is_err());
        for bad in ["[wallpaper]\nversion=2\n", "[wallpaper.grants]\nevil=1\n"] {
            let settings: crate::settings::Settings = toml::from_str(bad).unwrap();
            assert_eq!(settings.wallpaper, WallpaperSettings::default(), "{bad}");
        }
        // A [wallpaper] section that cannot be read costs only itself.
        let kept: crate::settings::Settings = toml::from_str("[clipboard]\ntext = false\n\n[wallpaper]\nversion = 9\nfuture = true\n").unwrap();
        assert_eq!(kept.wallpaper, WallpaperSettings::default());
        assert!(!kept.clipboard.text);
        // Any revision a grant can get must survive a trip through settings.toml.
        let mut settings = crate::settings::Settings::default();
        settings.wallpaper.grants.insert("a".repeat(64), i64::MAX as u64);
        let text = toml::to_string(&settings).unwrap();
        assert_eq!(toml::from_str::<crate::settings::Settings>(&text).unwrap().wallpaper, settings.wallpaper);
    }
}
