//! Location profiles: named snapshots of the layout, so a laptop that sits
//! left of the desk monitor at work and below the TV at home switches with
//! one command, or by itself when it joins that network.
//!
//! A profile holds, for every paired computer when it was saved, its side
//! and alignment (`devices.toml`'s `side` and `align`). Nothing else: edge
//! rules, gaming mode and the rest of `settings.toml` describe how you like
//! crossing to feel, not where the computers stand, so they stay the same
//! everywhere. Putting a profile in place goes through
//! [`manage::place`](crate::manage::place), like `one-mk layout`: each
//! computer it moves gets a new layout version, so the other computer
//! adopts the mirror image (newest change wins).
//!
//! Profiles live in `profiles.toml` next to `devices.toml`, written through
//! a file of its own renamed over it, under an exclusive lock (the same
//! scheme as `Settings::update`), so two commands at once both land.
//!
//! Automatic switching (`one-mk profile auto`): a profile may name Wi-Fi
//! networks (SSIDs) and networks (a router's MAC address, or a subnet this
//! computer has an address in). The daemon listens for network changes
//! (see [`crate::netwatch`]; nothing is polled) and puts the first profile
//! that matches in place, once per change of network: tweaks made with
//! `one-mk layout` afterwards stand until the network changes again (also
//! across a restart, if the network is still the same profile's). Set
//! automatic switching on one of the two computers (or give both mirror
//! images): both switching at once is harmless, the newer change wins, but
//! it is pointless.

use crate::control::{self, Request};
use crate::manage;
use anyhow::{Context, Result, bail};
use one_mk_net::TrustStore;
use one_mk_proto::{Align, Side, sanitize_name};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

const FILE: &str = "profiles.toml";
/// Held while profiles are changed (see [`Profiles::update`]).
const LOCK_FILE: &str = "profiles.toml.lock";

/// Everything in `profiles.toml`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Profiles {
    /// The profile last put in place, by `one-mk profile use` or
    /// automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    #[serde(default, rename = "profile")]
    pub list: Vec<Profile>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    /// Switch to it automatically on these Wi-Fi networks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ssid: Vec<String>,
    /// ... or on these networks: a router's MAC address, or a subnet
    /// (`192.168.1.0/24`) this computer has an address in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network: Vec<String>,
    #[serde(default, rename = "device")]
    pub devices: Vec<Placed>,
}

/// Where one paired computer stands in a profile.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Placed {
    pub fingerprint: String,
    /// Its name when saved, for people reading the file.
    #[serde(default)]
    pub name: String,
    pub side: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub align: String,
}

/// What a network-change notification found out about the networks this
/// computer is on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkId {
    /// Wi-Fi networks this computer is connected to.
    pub ssids: Vec<String>,
    /// The MAC addresses of the default routers, lowercase with colons.
    pub gateways: Vec<String>,
    /// This computer's addresses.
    pub addrs: Vec<IpAddr>,
}

/// A `--network` rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// A router's MAC address.
    Gateway([u8; 6]),
    /// A subnet: a network address and prefix length.
    Subnet(IpAddr, u8),
}

impl std::str::FromStr for Rule {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let hex: Vec<&str> = s.split([':', '-']).collect();
        if hex.len() == 6 && hex.iter().all(|h| h.len() == 2) {
            let mut mac = [0u8; 6];
            for (b, h) in mac.iter_mut().zip(&hex) {
                *b = u8::from_str_radix(h, 16).map_err(|_| format!("{s:?} is not a MAC address"))?;
            }
            return Ok(Rule::Gateway(mac));
        }
        let wanted = || format!("expected a router's MAC address (aa:bb:cc:dd:ee:ff) or a subnet (192.168.1.0/24), not {s:?}");
        let (ip, len) = s.split_once('/').ok_or_else(wanted)?;
        let ip: IpAddr = ip.parse().map_err(|_| wanted())?;
        let len: u8 = len.parse().map_err(|_| wanted())?;
        let max = if ip.is_ipv4() { 32 } else { 128 };
        if len > max {
            return Err(format!("{s:?}: a prefix is at most {max} bits"));
        }
        Ok(Rule::Subnet(ip, len))
    }
}

impl Rule {
    fn matches(&self, net: &NetworkId) -> bool {
        match *self {
            Rule::Gateway(mac) => net.gateways.iter().any(|g| *g == format_mac(mac)),
            Rule::Subnet(base, len) => net.addrs.iter().any(|a| in_subnet(*a, base, len)),
        }
    }
}

pub fn format_mac(mac: [u8; 6]) -> String {
    mac.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

fn in_subnet(addr: IpAddr, base: IpAddr, len: u8) -> bool {
    let mask = |bits: u32, len: u8| if len == 0 { 0 } else { u128::MAX << (bits - u32::from(len)) };
    match (addr, base) {
        (IpAddr::V4(a), IpAddr::V4(b)) => {
            let m = mask(32, len) as u32;
            u32::from(a) & m == u32::from(b) & m
        }
        (IpAddr::V6(a), IpAddr::V6(b)) => {
            let m = mask(128, len);
            u128::from(a) & m == u128::from(b) & m
        }
        _ => false,
    }
}

impl Profile {
    /// Whether it switches by itself.
    pub fn is_auto(&self) -> bool {
        !self.ssid.is_empty() || !self.network.is_empty()
    }

    fn matches(&self, net: &NetworkId) -> bool {
        self.ssid.iter().any(|s| net.ssids.contains(s))
            || self.network.iter().filter_map(|r| r.parse::<Rule>().ok()).any(|r| r.matches(net))
    }
}

fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// A profile name that is safe to show and to put on one status line.
pub fn check_name(name: &str) -> Result<String> {
    let clean = sanitize_name(name);
    if name.trim().is_empty() || clean != name.trim() || name.contains('\t') {
        bail!("a profile name is up to 64 letters, digits, spaces and punctuation; {name:?} is not");
    }
    Ok(clean)
}

impl Profiles {
    pub fn load(dir: &Path) -> Result<Profiles> {
        let path = path(dir);
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("reading {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Profiles::default()),
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self, dir: &Path) -> Result<()> {
        static N: AtomicU32 = AtomicU32::new(0);
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{FILE}.{}.{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let write = || -> Result<()> {
            std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
            std::fs::rename(&tmp, path(dir))?;
            Ok(())
        };
        write().inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    /// Changes the saved profiles: waits for an exclusive lock on a file
    /// next to `profiles.toml`, loads what is there, applies `change` and
    /// saves. If `change` fails nothing is written.
    pub fn update<R>(dir: &Path, change: impl FnOnce(&mut Profiles) -> Result<R>) -> Result<R> {
        std::fs::create_dir_all(dir)?;
        let lock_path = dir.join(LOCK_FILE);
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("opening {}", lock_path.display()))?;
        lock.lock().with_context(|| format!("locking {}", lock_path.display()))?;
        // Unlocked when `lock` is dropped (also if this fails or panics).
        let mut profiles = Profiles::load(dir)?;
        let out = change(&mut profiles)?;
        profiles.save(dir)?;
        Ok(out)
    }

    pub fn get(&self, name: &str) -> Option<&Profile> {
        self.list.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut Profile> {
        self.list.iter_mut().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn names(&self) -> Vec<String> {
        self.list.iter().map(|p| p.name.clone()).collect()
    }

    pub fn any_auto(&self) -> bool {
        self.list.iter().any(Profile::is_auto)
    }

    /// The first profile (in file order) whose rules match `net`.
    pub fn matching(&self, net: &NetworkId) -> Option<&Profile> {
        self.list.iter().find(|p| p.matches(net))
    }
}

/// Every paired computer's side and alignment now.
pub fn snapshot(trust: &TrustStore) -> Vec<Placed> {
    trust
        .devices()
        .iter()
        .map(|d| Placed {
            fingerprint: d.fingerprint.clone(),
            name: d.name.clone(),
            side: d.side().to_string(),
            align: if d.align() == Align::Stretch { String::new() } else { d.align().to_string() },
        })
        .collect()
}

/// Puts `profile`'s layout in `devices.toml`. Computers paired since it was
/// saved are left where they are; those no longer paired are skipped.
/// Returns the names of the computers that moved.
pub fn apply(dir: &Path, profile: &Profile) -> Result<Vec<String>> {
    let mut trust = TrustStore::load(dir)?;
    let mut moved = Vec::new();
    for p in &profile.devices {
        let Some(fp) = p.fingerprint.parse().ok() else { continue };
        let Some(dev) = trust.get_mut(&fp) else { continue };
        let side: Side = p.side.parse().unwrap_or_else(|_| dev.side());
        let align: Align = p.align.parse().unwrap_or_default();
        if (dev.side(), dev.align()) != (side, align) {
            manage::place(dev, side, align);
            moved.push(sanitize_name(&dev.name));
        }
    }
    if !moved.is_empty() {
        trust.save()?;
    }
    Ok(moved)
}

/// Puts profile `name` in place and records it as the active one. Returns
/// the profile's name as saved and who moved.
pub fn use_profile(dir: &Path, name: &str) -> Result<(String, Vec<String>)> {
    Profiles::update(dir, |all| {
        let Some(p) = all.get(name).cloned() else { bail!("no layout profile called {name:?}; see `one-mk profile list`") };
        let moved = apply(dir, &p)?;
        all.active = Some(p.name.clone());
        Ok((p.name, moved))
    })
}

/// `one-mk profile ...`
#[derive(clap::Subcommand, Debug)]
pub enum ProfileCmd {
    /// Save where every paired computer is now as a profile (replacing one
    /// of the same name, keeping its automatic switching).
    Save { name: String },
    /// Put a saved profile's layout in place (applies at once if one-mk is
    /// running; the other computers pick it up too).
    Use { name: String },
    /// List the saved profiles.
    List,
    /// Delete a profile.
    Delete { name: String },
    /// Switch to a profile by itself when this computer joins a Wi-Fi
    /// network or a network (needs NetworkManager on Linux).
    Auto {
        name: String,
        /// A Wi-Fi network name (repeatable).
        #[arg(long, value_name = "WIFI-NAME")]
        ssid: Vec<String>,
        /// A router's MAC address (aa:bb:cc:dd:ee:ff) or a subnet
        /// (192.168.1.0/24) (repeatable).
        #[arg(long, value_name = "GATEWAY-MAC|SUBNET")]
        network: Vec<String>,
        /// Stop switching to it by itself.
        #[arg(long, conflicts_with_all = ["ssid", "network"])]
        off: bool,
    },
}

pub async fn run(dir: &Path, cmd: ProfileCmd) -> Result<()> {
    match cmd {
        ProfileCmd::Save { name } => {
            let name = check_name(&name)?;
            let trust = TrustStore::load(dir)?;
            let devices = snapshot(&trust);
            if devices.is_empty() {
                bail!("no computers are paired yet, so there is no layout to save");
            }
            let n = devices.len();
            let replaced = Profiles::update(dir, |all| {
                let replaced = match all.get_mut(&name) {
                    Some(p) => {
                        p.devices = devices;
                        true
                    }
                    None => {
                        all.list.push(Profile { name: name.clone(), devices, ..Profile::default() });
                        false
                    }
                };
                all.active = Some(name.clone());
                Ok(replaced)
            })?;
            let what = if n == 1 { "1 computer".to_owned() } else { format!("{n} computers") };
            println!("{} layout profile {name:?} ({what}).", if replaced { "Updated" } else { "Saved" });
            Ok(())
        }
        ProfileCmd::Use { name } => {
            let (name, moved) = use_profile(dir, &name)?;
            if moved.is_empty() {
                println!("Using layout profile {name:?}; everything was already in place.");
                return Ok(());
            }
            println!("Using layout profile {name:?}: moved {}.", moved.join(", "));
            if control::request(dir, Request::Reload).await?.is_some() {
                println!("Applied right away. The other computers pick up the change too.");
            } else {
                println!("one-mk is not running; this takes effect the next time it starts.");
            }
            Ok(())
        }
        ProfileCmd::List => {
            let all = Profiles::load(dir)?;
            if all.list.is_empty() {
                println!("No layout profiles yet. Save the current layout with `one-mk profile save <name>`.");
                return Ok(());
            }
            print!("{}", render(&all));
            Ok(())
        }
        ProfileCmd::Delete { name } => {
            let gone = Profiles::update(dir, |all| {
                let Some(i) = all.list.iter().position(|p| p.name.eq_ignore_ascii_case(&name)) else {
                    bail!("no layout profile called {name:?}; see `one-mk profile list`")
                };
                let gone = all.list.remove(i).name;
                if all.active.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(&gone)) {
                    all.active = None;
                }
                Ok(gone)
            })?;
            println!("Deleted layout profile {gone:?}.");
            Ok(())
        }
        ProfileCmd::Auto { name, ssid, network, off } => {
            if !off && ssid.is_empty() && network.is_empty() {
                bail!("say when to switch to {name:?}: --ssid <wifi-name> and/or --network <gateway-mac|subnet>, or --off");
            }
            let mut rules = Vec::new();
            for n in &network {
                let r: Rule = n.parse().map_err(anyhow::Error::msg)?;
                rules.push(match r {
                    Rule::Gateway(mac) => format_mac(mac),
                    Rule::Subnet(ip, len) => format!("{ip}/{len}"),
                });
            }
            let ssid: Vec<String> = ssid.into_iter().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
            let p = Profiles::update(dir, |all| {
                let Some(p) = all.get_mut(&name) else { bail!("no layout profile called {name:?}; save one first") };
                p.ssid = ssid;
                p.network = rules;
                Ok(p.clone())
            })?;
            match p.is_auto() {
                true => println!("Switching to {:?} by itself {}.", p.name, describe_rules(&p)),
                false => println!("{:?} no longer switches by itself.", p.name),
            }
            if !netwatch_available() {
                println!("{}", crate::netwatch::UNSUPPORTED);
            }
            if p.is_auto() && control::request(dir, Request::Reload).await?.is_none() {
                println!("one-mk is not running; it starts watching the network when it starts.");
            }
            Ok(())
        }
    }
}

fn netwatch_available() -> bool {
    cfg!(all(unix, not(target_os = "macos")))
}

fn describe_rules(p: &Profile) -> String {
    let mut parts: Vec<String> = p.ssid.iter().map(|s| format!("on Wi-Fi {s:?}")).collect();
    parts.extend(p.network.iter().map(|n| match n.contains('/') {
        true => format!("on {n}"),
        false => format!("behind the router {n}"),
    }));
    parts.join(" or ")
}

fn render(all: &Profiles) -> String {
    let mut out = String::new();
    for p in &all.list {
        let active = all.active.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(&p.name));
        out += &format!("{}{}\n", p.name, if active { " (in use)" } else { "" });
        if p.is_auto() {
            out += &format!("  switches by itself {}\n", describe_rules(p));
        }
        for d in &p.devices {
            let side = d.side.parse().unwrap_or(Side::Right);
            out += &format!("  {:<24} {}\n", sanitize_name(&d.name), manage::placement(side, d.align.parse().unwrap_or_default()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_net::{Device, Identity};

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("one-mk-profiles-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn pair(dir: &Path, name: &str, side: Side) -> String {
        let id = Identity::ephemeral().unwrap();
        let mut t = TrustStore::load(dir).unwrap();
        t.upsert(Device { name: name.into(), fingerprint: id.fingerprint.to_string(), side: side.to_string(), ..Device::default() });
        t.save().unwrap();
        id.fingerprint.to_string()
    }

    fn place_of(dir: &Path, fp: &str) -> (Side, Align, u64) {
        let t = TrustStore::load(dir).unwrap();
        let d = t.find(&fp[..8]).unwrap();
        (d.side(), d.align(), d.layout_version)
    }

    /// Save at home, rearrange for work, save that, then switch back and
    /// forth: each switch restores every computer, with a newer version.
    #[tokio::test]
    async fn save_and_use_round_trip() {
        let dir = temp_dir("round");
        let laptop = pair(&dir, "laptop", Side::Left);
        let tv = pair(&dir, "tv", Side::Top);
        manage::set_layout(&dir, &laptop[..8], None, Some(Align::Offset(-120)), None).unwrap();
        run(&dir, ProfileCmd::Save { name: "home".into() }).await.unwrap();
        manage::set_layout(&dir, &laptop[..8], Some(Side::Bottom), Some(Align::Center), None).unwrap();
        manage::set_layout(&dir, &tv[..8], Some(Side::Right), None, None).unwrap();
        run(&dir, ProfileCmd::Save { name: "work".into() }).await.unwrap();

        let before = place_of(&dir, &laptop).2;
        let (name, moved) = use_profile(&dir, "HOME").unwrap();
        assert_eq!(name, "home");
        assert_eq!(moved, vec!["laptop".to_owned(), "tv".to_owned()]);
        let (side, align, version) = place_of(&dir, &laptop);
        assert_eq!((side, align), (Side::Left, Align::Offset(-120)));
        assert!(version > before, "a profile is a new change: it wins against the peer's older one");
        assert_eq!(place_of(&dir, &tv).0, Side::Top);
        assert_eq!(Profiles::load(&dir).unwrap().active.as_deref(), Some("home"));
        // Already in place: nothing moves, no new version.
        assert!(use_profile(&dir, "home").unwrap().1.is_empty());
        assert_eq!(place_of(&dir, &laptop).2, version);

        use_profile(&dir, "work").unwrap();
        assert_eq!(place_of(&dir, &laptop).0, Side::Bottom);
        assert_eq!(place_of(&dir, &laptop).1, Align::Center);
        assert_eq!(place_of(&dir, &tv).0, Side::Right);

        // A computer paired later stays put; one forgotten is skipped.
        let desk = pair(&dir, "desk", Side::Right);
        TrustStore::load(&dir).map(|mut t| { t.remove("tv"); t.save().unwrap() }).unwrap();
        use_profile(&dir, "home").unwrap();
        assert_eq!(place_of(&dir, &desk).0, Side::Right);
        assert!(use_profile(&dir, "nowhere").is_err());

        let text = render(&Profiles::load(&dir).unwrap());
        assert!(text.contains("home (in use)") && text.contains("on your left, its top edge 120 px above yours"), "{text}");
        run(&dir, ProfileCmd::Delete { name: "home".into() }).await.unwrap();
        let all = Profiles::load(&dir).unwrap();
        assert_eq!((all.names(), all.active), (vec!["work".to_owned()], None));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Many processes' worth of writers at once: every change lands.
    #[test]
    fn concurrent_writes_all_land() {
        let dir = temp_dir("concurrent");
        let threads: Vec<_> = (0..16)
            .map(|i| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    for j in 0..5 {
                        Profiles::update(&dir, |all| {
                            all.list.push(Profile { name: format!("p{i}-{j}"), ..Profile::default() });
                            Ok(())
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let all = Profiles::load(&dir).unwrap();
        assert_eq!(all.list.len(), 80);
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rules_parse_and_match() {
        let net = NetworkId {
            ssids: vec!["Home Wi-Fi".into()],
            gateways: vec!["aa:bb:cc:00:11:22".into()],
            addrs: vec!["192.168.1.37".parse().unwrap(), "fd00::5".parse().unwrap()],
        };
        let rule = |s: &str| s.parse::<Rule>().unwrap();
        assert!(rule("AA-BB-CC-00-11-22").matches(&net));
        assert!(!rule("aa:bb:cc:00:11:23").matches(&net));
        assert!(rule("192.168.1.0/24").matches(&net));
        assert!(rule("192.168.0.0/16").matches(&net));
        assert!(!rule("192.168.2.0/24").matches(&net));
        assert!(rule("0.0.0.0/0").matches(&net));
        assert!(rule("fd00::/8").matches(&net));
        assert!(!rule("fe80::/10").matches(&net));
        for bad in ["192.168.1.1", "10.0.0.0/33", "aa:bb:cc", "zz:bb:cc:00:11:22", "home"] {
            assert!(bad.parse::<Rule>().is_err(), "{bad}");
        }
        let profiles = Profiles {
            active: None,
            list: vec![
                Profile { name: "manual".into(), ..Profile::default() },
                Profile { name: "office".into(), network: vec!["10.0.0.0/8".into()], ..Profile::default() },
                Profile { name: "home".into(), ssid: vec!["Home Wi-Fi".into()], ..Profile::default() },
                Profile { name: "home too".into(), network: vec!["192.168.1.0/24".into()], ..Profile::default() },
            ],
        };
        assert_eq!(profiles.matching(&net).map(|p| p.name.as_str()), Some("home"), "the first that matches");
        assert_eq!(profiles.matching(&NetworkId::default()), None);
        assert!(profiles.any_auto());
        assert!(check_name("home office").is_ok());
        assert!(check_name("tab\there").is_err());
        assert!(check_name("  ").is_err());
    }

    /// `profile auto` checks its rules and needs an existing profile.
    #[tokio::test]
    async fn auto_rules_are_checked() {
        let dir = temp_dir("auto");
        pair(&dir, "laptop", Side::Left);
        run(&dir, ProfileCmd::Save { name: "home".into() }).await.unwrap();
        let auto = |name: &str, ssid: &[&str], network: &[&str], off| ProfileCmd::Auto {
            name: name.into(),
            ssid: ssid.iter().map(|s| s.to_string()).collect(),
            network: network.iter().map(|s| s.to_string()).collect(),
            off,
        };
        assert!(run(&dir, auto("home", &[], &["192.168.1.1"], false)).await.is_err());
        assert!(run(&dir, auto("nope", &["x"], &[], false)).await.is_err());
        assert!(run(&dir, auto("home", &[], &[], false)).await.is_err());
        run(&dir, auto("home", &["Home"], &["AA-BB-CC-00-11-22", "192.168.1.7/24"], false)).await.unwrap();
        let p = Profiles::load(&dir).unwrap().get("home").cloned().unwrap();
        assert_eq!((p.ssid, p.network), (vec!["Home".to_owned()], vec!["aa:bb:cc:00:11:22".to_owned(), "192.168.1.7/24".to_owned()]));
        run(&dir, auto("home", &[], &[], true)).await.unwrap();
        assert!(!Profiles::load(&dir).unwrap().any_auto());
        let _ = std::fs::remove_dir_all(dir);
    }
}
