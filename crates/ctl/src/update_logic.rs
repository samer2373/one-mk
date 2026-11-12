//! Bounded, platform-independent signed-release policy. No I/O or crypto backend.

use anyhow::{Context, Result, bail, ensure};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

pub const MANIFEST_LIMIT: usize = 256 * 1024;
pub const ASSET_LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub os: String,
    pub arch: String,
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: String,
    pub release_time: String,
    pub sequence: u64,
    pub minimum_supported_version: String,
    pub assets: Vec<Asset>,
    #[serde(default)]
    pub revoked: Vec<String>,
    #[serde(default)]
    pub next_key: Option<String>,
}

pub fn version(text: &str) -> Result<Version> {
    ensure!(text.len() <= 128, "release version is too long");
    Version::parse(text).context("invalid release version; ask the publisher to repair the manifest")
}

pub fn compare(left: &str, right: &str) -> Result<Ordering> {
    Ok(version(left)?.cmp_precedence(&version(right)?))
}

pub fn hex_bytes<const LENGTH: usize>(text: &str) -> Result<[u8; LENGTH]> {
    ensure!(text.len() == LENGTH * 2, "invalid hexadecimal length");
    let mut bytes = [0; LENGTH];
    for (slot, pair) in bytes.iter_mut().zip(text.as_bytes().as_chunks::<2>().0) {
        let digit = |byte: u8| match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            b'a'..=b'f' => Ok(byte - b'a' + 10),
            _ => bail!("invalid lowercase hexadecimal value"),
        };
        *slot = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Ok(bytes)
}

pub fn signature_inputs(key: &[u8], signature: &[u8], message: &[u8]) -> Result<()> {
    ensure!(key.len() == 32 && signature.len() == 64 && !message.is_empty() && message.len() <= MANIFEST_LIMIT,
        "invalid signature, key or manifest size; this release cannot be trusted");
    Ok(())
}

pub fn parse(bytes: &[u8]) -> Result<Manifest> {
    ensure!(!bytes.is_empty() && bytes.len() <= MANIFEST_LIMIT, "release manifest is empty or exceeds 256 KiB");
    let manifest: Manifest = serde_json::from_slice(bytes).context("invalid update manifest; ask the publisher to repair this release")?;
    version(&manifest.version)?;
    version(&manifest.minimum_supported_version)?;
    ensure!(manifest.sequence > 0, "release sequence must be positive");
    ensure!(manifest.release_time.len() == 20 && manifest.release_time.ends_with('Z') &&
        manifest.release_time.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-', 10 => byte == b'T', 13 | 16 => byte == b':', 19 => byte == b'Z', _ => byte.is_ascii_digit(),
        }), "invalid UTC release time");
    let number = |range: std::ops::Range<usize>| -> Result<u32> {
        Ok(manifest.release_time.get(range).context("invalid UTC release time")?.parse()?)
    };
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month { 1 | 3 | 5 | 7 | 8 | 10 | 12 => 31, 4 | 6 | 9 | 11 => 30, 2 if leap => 29, 2 => 28, _ => 0 };
    ensure!(year > 0 && day > 0 && day <= days && number(11..13)? < 24 && number(14..16)? < 60 && number(17..19)? < 60,
        "invalid UTC release time");
    ensure!(compare(&manifest.minimum_supported_version, &manifest.version)? != Ordering::Greater,
        "minimum supported version is newer than the release");
    ensure!(!manifest.assets.is_empty() && manifest.assets.len() <= 128 && manifest.revoked.len() <= 128, "invalid release asset or revocation count");
    let mut names = std::collections::HashSet::new();
    let mut selections = std::collections::HashSet::new();
    for asset in &manifest.assets {
        ensure!(!asset.name.is_empty() && asset.name.len() <= 160 && asset.name.bytes().all(|byte|
            byte.is_ascii_alphanumeric() || b"._-".contains(&byte)) && !asset.name.contains("..") && !asset.name.starts_with('.'),
            "unsafe release asset name");
        ensure!(names.insert(&asset.name), "duplicate release asset name");
        ensure!(asset.size > 0 && asset.size <= ASSET_LIMIT, "release asset size exceeds safety limits");
        hex_bytes::<32>(&asset.sha256)?;
        ensure!(["linux", "macos", "windows", "any"].contains(&asset.os.as_str()) &&
            ["x86_64", "aarch64", "any"].contains(&asset.arch.as_str()) &&
            ["binary", "appimage", "deb", "installer", "app", "archive", "metadata"].contains(&asset.kind.as_str()), "unsupported release asset platform or kind");
        if matches!(asset.kind.as_str(), "binary" | "appimage") {
            ensure!(selections.insert((&asset.os, &asset.arch, &asset.kind)), "ambiguous executable assets");
        }
    }
    for revoked in &manifest.revoked { version(revoked)?; }
    if let Some(key) = &manifest.next_key { hex_bytes::<32>(key)?; }
    Ok(manifest)
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub sequence: u64,
    pub digest: String,
    pub highest_version: Option<String>,
    pub last_attempt: u64,
    pub available: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn startup_due(last_attempt: u64, now: u64) -> bool {
    now.saturating_sub(last_attempt) >= 24 * 60 * 60
}

pub fn accept(manifest: &Manifest, current: &str, state: &State, digest: &str, floor: u64) -> Result<bool> {
    ensure!(manifest.sequence >= floor && manifest.sequence >= state.sequence, "older signed release refused; ask the publisher for a higher release sequence");
    ensure!(manifest.sequence != state.sequence || state.digest == digest, "release sequence reused with different contents; ask the publisher to repair it");
    ensure!(compare(&manifest.version, current)? != Ordering::Less, "release downgrade refused; install an authenticated newer release");
    if let Some(highest) = &state.highest_version {
        ensure!(compare(&manifest.version, highest)? != Ordering::Less, "release is older than a previously accepted update");
    }
    ensure!(compare(current, &manifest.minimum_supported_version)? != Ordering::Less,
        "this installation is too old for automatic updating; download and install the complete release manually");
    ensure!(!manifest.revoked.iter().any(|revoked| compare(revoked, &manifest.version).is_ok_and(|order| order == Ordering::Equal)),
        "this release was revoked; wait for a replacement release");
    Ok(compare(&manifest.version, current)? == Ordering::Greater)
}

pub fn select<'a>(manifest: &'a Manifest, os: &str, arch: &str, kind: &str) -> Result<&'a Asset> {
    let mut matching = manifest.assets.iter().filter(|asset| asset.os == os && asset.arch == arch && asset.kind == kind);
    let asset = matching.next().context("no matching update for this installation; download the complete release manually")?;
    ensure!(matching.next().is_none(), "ambiguous update assets; ask the publisher to repair the release");
    Ok(asset)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallKind { Portable, AppImage, Deb, Homebrew, Flatpak, WindowsInstaller, MacApp, Managed }

impl InstallKind {
    pub fn automatic(self) -> bool { matches!(self, Self::Portable | Self::AppImage | Self::MacApp) }

    /// Whether the window can do the whole update: the ones that replace
    /// themselves, and a Linux package, which the update installs after
    /// the system asks for the password (never without asking).
    pub fn installs_in_app(self) -> bool { self.automatic() || self == Self::Deb }

    /// Whether the install asks for the account's password.
    pub fn needs_password(self) -> bool { self == Self::Deb }

    pub fn advice(self) -> &'static str {
        match self {
            Self::Portable | Self::AppImage | Self::MacApp => "Ready to update in place",
            Self::Deb => "Installing asks for your password once",
            Self::Homebrew | Self::Flatpak => "Updates come with your package manager",
            Self::WindowsInstaller => "Download the new Windows installer to update this installation",
            Self::Managed => "Download the complete release to update this installation",
        }
    }
}

pub fn select_install<'a>(manifest: &'a Manifest, os: &str, arch: &str, kind: InstallKind) -> Result<&'a Asset> {
    ensure!((kind != InstallKind::MacApp || os == "macos") && (kind != InstallKind::AppImage || os == "linux"),
        "This update doesn't match your operating system");
    if kind == InstallKind::MacApp {
        let mut matching = manifest.assets.iter().filter(|asset| asset.os == os && asset.arch == arch && asset.kind == "app" && asset.name.ends_with(".app.zip"));
        let asset = matching.next().context("No app update is available for this computer yet")?;
        ensure!(matching.next().is_none(), "The release contains more than one matching app update");
        return Ok(asset);
    }
    ensure!(kind.installs_in_app(), "{}", kind.advice());
    if kind == InstallKind::Deb {
        ensure!(os == "linux", "This update doesn't match your operating system");
        return select(manifest, os, arch, "deb");
    }
    select(manifest, os, arch, if kind == InstallKind::AppImage { "appimage" } else { "binary" })
}

pub fn translocated(path: &str) -> bool {
    let path = path.replace('\\', "/");
    path.contains("/AppTranslocation/") || path.starts_with("/Volumes/")
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct UpdateStatus {
    pub check: bool,
    pub install_automatically: bool,
    pub available: Option<String>,
    pub error: Option<String>,
    pub can_update: bool,
    /// Installing asks for the account's password (a Linux package), so it
    /// is never done without the person asking.
    #[serde(default)]
    pub needs_password: bool,
    pub move_to_applications: bool,
    pub rollback_available: bool,
    pub advice: String,
    /// A version the person went back from: it isn't installed by itself
    /// again (Update now still offers it).
    #[serde(default)]
    pub skipped: Option<String>,
}

/// Where the version a rollback went back from is kept: a file of its own,
/// so the older version that runs after the rollback, which doesn't know
/// this field, still reads the update state.
pub const SKIPPED: &str = "update-skipped";

pub fn skipped_version(home: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(home.join(SKIPPED)).ok()?;
    let v = text.trim();
    (!v.is_empty() && v.len() <= 32 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')).then(|| v.to_owned())
}

pub fn install_kind(path: &str, home: &str, appimage: bool, flatpak: bool) -> InstallKind {
    let path = path.replace('\\', "/");
    let lower = path.to_ascii_lowercase();
    if flatpak || path.starts_with("/app/") { return InstallKind::Flatpak; }
    if lower.contains("/program files/") || lower.contains("/program files (x86)/") || lower.contains("/windowsapps/") ||
        lower.ends_with("/appdata/local/programs/one-mk/one-mk.exe") { return InstallKind::WindowsInstaller; }
    if lower.contains(".app/") { return InstallKind::MacApp; }
    if path.starts_with("/opt/homebrew/") || path.starts_with("/usr/local/Cellar/") || path.starts_with("/usr/local/Caskroom/") || path.contains("/.linuxbrew/") { return InstallKind::Homebrew; }
    if path.starts_with("/usr/") { return InstallKind::Deb; }
    if path.starts_with("/opt/") { return InstallKind::Managed; }
    let home = home.replace('\\', "/");
    if !home.is_empty() && (path.starts_with(&format!("{home}/")) || (home.contains(':') && lower.starts_with(&format!("{home}/").to_ascii_lowercase()))) {
        // The caller verified this process runs from that AppImage: whatever
        // the file is called, it is the AppImage (window, tray, libraries).
        if appimage { return InstallKind::AppImage; }
        // Not one built or installed by cargo: those belong to cargo.
        let cargo = lower.contains("/.cargo/") || lower.contains("/target/debug/") || lower.contains("/target/release/")
            || lower.contains("-apple-darwin/") || lower.contains("-pc-windows-") || lower.contains("-unknown-linux-");
        if !cargo && (path.ends_with("/one-mk") || lower.ends_with("/one-mk.exe")) { return InstallKind::Portable; }
    }
    InstallKind::Managed
}

pub fn cached_version(home: &std::path::Path, current: &str) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(home.join("update-state.json")).ok()?;
    let mut bytes = Vec::new();
    file.take(16 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 16 * 1024 { return None; }
    let state: State = serde_json::from_slice(&bytes).ok()?;
    let available = state.available?;
    (compare(&available, current).ok()? == Ordering::Greater).then_some(available)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skipped_version_is_read_only_when_it_looks_like_one() {
        let dir = std::env::temp_dir().join(format!("omk-skip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(skipped_version(&dir), None);
        std::fs::write(dir.join(SKIPPED), "0.2.0\n").unwrap();
        assert_eq!(skipped_version(&dir).as_deref(), Some("0.2.0"));
        std::fs::write(dir.join(SKIPPED), "0.2.0; rm -rf").unwrap();
        assert_eq!(skipped_version(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn fixture() -> Manifest {
        Manifest { version: "0.1.13".into(), release_time: "2026-10-30T00:00:00Z".into(), sequence: 2,
            minimum_supported_version: "0.1.0".into(), assets: vec![Asset { name: "one-mk-linux".into(), size: 100,
                sha256: "00".repeat(32), os: "linux".into(), arch: "x86_64".into(), kind: "binary".into() }], revoked: vec![], next_key: None }
    }

    #[test]
    fn versions_and_replay() -> Result<()> {
        assert_eq!(compare("0.1.9", "0.1.10")?, Ordering::Less);
        assert_eq!(compare("1.0.0-rc.1", "1.0.0")?, Ordering::Less);
        assert_eq!(compare("1.0.0+one", "1.0.0+two")?, Ordering::Equal);
        let mut manifest = fixture();
        let state = State { sequence: 2, digest: "same".into(), highest_version: Some("0.1.13".into()), ..Default::default() };
        assert!(accept(&manifest, "0.1.12", &state, "same", 1)?);
        assert!(!accept(&manifest, "0.1.13", &state, "same", 1)?);
        assert!(accept(&manifest, "0.1.12", &state, "changed", 1).is_err());
        manifest.sequence = 1;
        assert!(accept(&manifest, "0.1.12", &state, "same", 1).is_err());
        manifest.sequence = 3;
        manifest.version = "0.1.11".into();
        assert!(accept(&manifest, "0.1.12", &state, "same", 1).is_err());
        assert!(accept(&manifest, "0.1.10", &state, "same", 1).is_err());
        manifest = fixture();
        manifest.revoked = vec![manifest.version.clone()];
        assert!(accept(&manifest, "0.1.12", &State::default(), "same", 0).is_err());
        manifest.revoked.clear();
        manifest.minimum_supported_version = "0.1.13".into();
        assert!(accept(&manifest, "0.1.12", &State::default(), "same", 0).is_err());
        manifest.minimum_supported_version = "0.1.0".into();
        assert!(accept(&manifest, "0.1.12", &State::default(), "same", 3).is_err());
        Ok(())
    }

    #[test]
    fn hostile_manifests_and_selection() -> Result<()> {
        let fixture = serde_json::to_vec(&fixture())?;
        assert!(parse(&fixture).is_ok());
        for bad in [vec![], vec![b' '; MANIFEST_LIMIT + 1], b"{\"sequence\":1,\"sequence\":2}".to_vec(),
            format!("{}0{}", "[".repeat(200), "]".repeat(200)).into_bytes()] { assert!(parse(&bad).is_err()); }
        for name in ["../x", "a/b", "a\\b", ".hidden", "a\n", ""] {
            let mut manifest = super::tests::fixture();
            manifest.assets[0].name = name.into();
            assert!(parse(&serde_json::to_vec(&manifest)?).is_err());
        }
        let text = String::from_utf8(fixture)?;
        for bad in [text.replace("\"size\":100", "\"size\":-1"), text.replace("\"sequence\":2", "\"sequence\":\"2\""),
            text.replace("\"sequence\":2", "\"sequence\":2,\"sequence\":2"), text.replace("\"size\":100", "\"size\":100,\"size\":100")] { assert!(parse(bad.as_bytes()).is_err()); }
        let manifest = super::tests::fixture();
        assert!(select(&manifest, "linux", "x86_64", "binary").is_ok());
        assert!(select(&manifest, "macos", "aarch64", "binary").is_err());
        for date in ["2026-02-29T00:00:00Z", "2026-13-01T00:00:00Z", "2026-10-30T24:00:00Z", "2026-10-30T00:60:00Z"] {
            let mut changed = manifest.clone();
            changed.release_time = date.into();
            assert!(parse(&serde_json::to_vec(&changed)?).is_err());
        }
        for os in ["linux", "macos", "windows"] { for arch in ["x86_64", "aarch64"] {
            let mut changed = manifest.clone();
            changed.assets[0].os = os.into();
            changed.assets[0].arch = arch.into();
            assert!(select(&changed, os, arch, "binary").is_ok());
        }}
        Ok(())
    }

    /// Every installed oneMK has this parser. A list with a field it does not know (an expiry or freshness time,
    /// say) is refused outright, so such a field can never be added to the list the release makes: it would stop
    /// every deployed client from updating (docs/UPDATES.md, "Freshness for install.sh"). A fresh install.sh
    /// gets freshness from its own constants instead. Change this only once no client in use is older than the
    /// release that reads the new field.
    #[test]
    fn a_field_this_parser_does_not_know_is_refused_so_the_list_cannot_grow_one() -> Result<()> {
        let text = String::from_utf8(serde_json::to_vec(&fixture())?)?;
        assert!(parse(text.as_bytes()).is_ok());
        for field in ["\"not_after\":\"2027-01-01T00:00:00Z\"", "\"valid_until\":1", "\"expires\":null"] {
            let extended = text.replacen('{', &format!("{{{field},"), 1);
            assert!(parse(extended.as_bytes()).is_err(), "{field} was accepted");
        }
        // The one optional field the parser does know, and the two it needs no value for.
        let known = text.replace("\"next_key\":null", &format!("\"next_key\":\"{}\"", "00".repeat(32)));
        assert!(parse(known.as_bytes()).is_ok());
        let without_optional = text.replace(",\"revoked\":[]", "").replace(",\"next_key\":null", "");
        assert!(parse(without_optional.as_bytes()).is_ok());
        Ok(())
    }

    #[test]
    fn install_paths() {
        assert!(!startup_due(100_000, 100_000));
        assert!(!startup_due(100_000, 99_999));
        assert!(!startup_due(100_000, 186_399));
        assert!(startup_due(100_000, 186_400));
        for (path, home, expected) in [
            ("/usr/bin/one-mk", "/home/me", InstallKind::Deb),
            ("/opt/homebrew/bin/one-mk", "/Users/me", InstallKind::Homebrew),
            ("/Applications/one-mk.app/Contents/MacOS/one-mk", "/Users/me", InstallKind::MacApp),
            ("C:\\Program Files\\one-mk\\one-mk.exe", "C:\\Users\\me", InstallKind::WindowsInstaller),
            ("C:\\Users\\me\\AppData\\Local\\Programs\\one-mk\\one-mk.exe", "C:\\Users\\me", InstallKind::WindowsInstaller),
            ("/home/linuxbrew/.linuxbrew/Cellar/one-mk/0.1.12/bin/one-mk", "/home/me", InstallKind::Homebrew),
            ("/home/me/.local/bin/one-mk", "/home/me", InstallKind::Portable),
            ("/home/me-other/.local/bin/one-mk", "/home/me", InstallKind::Managed),
            ("/tmp/one-mk", "/home/me", InstallKind::Managed),
            ("/home/me/Downloads/one-mk-linux/one-mk", "/home/me", InstallKind::Portable),
            ("/home/me/.cargo/bin/one-mk", "/home/me", InstallKind::Managed),
            ("/home/me/src/one-mk/target/release/one-mk", "/home/me", InstallKind::Managed),
            ("/home/me/src/one-mk/target/x86_64-unknown-linux-gnu/release/one-mk", "/home/me", InstallKind::Managed),
            ("C:\\Users\\me\\.cargo\\bin\\one-mk.exe", "C:\\Users\\me", InstallKind::Managed),
        ] { assert_eq!(install_kind(path, home, false, false), expected); }
        assert_eq!(install_kind("/home/me/one-mk.AppImage", "/home/me", true, false), InstallKind::AppImage);
        // An AppImage keeps its kind under any name, but only inside the home folder.
        assert_eq!(install_kind("/home/me/.local/bin/one-mk", "/home/me", true, false), InstallKind::AppImage);
        assert_eq!(install_kind("/mnt/apps/one-mk.AppImage", "/home/me", true, false), InstallKind::Managed);
        assert_eq!(install_kind("/app/bin/one-mk", "/home/me", false, true), InstallKind::Flatpak);
        assert_eq!(install_kind("C:\\Users\\me\\AppData\\Local\\one-mk-portable\\one-mk.exe", "C:\\Users\\me", false, false), InstallKind::Portable);
        assert_eq!(install_kind("c:\\users\\ME\\AppData\\Local\\one-mk-portable\\one-mk.exe", "C:\\Users\\me", false, false), InstallKind::Portable);
    }

    #[test]
    fn a_linux_package_installs_from_its_signed_deb_only_on_linux() -> Result<()> {
        let mut manifest = fixture();
        manifest.assets.clear();
        manifest.assets.push(Asset { name: "one-mk-x86_64-unknown-linux-gnu.deb".into(), size: 1, sha256: "ab".repeat(32),
            os: "linux".into(), arch: "x86_64".into(), kind: "deb".into() });
        assert_eq!(select_install(&manifest, "linux", "x86_64", InstallKind::Deb)?.name, "one-mk-x86_64-unknown-linux-gnu.deb");
        assert!(select_install(&manifest, "macos", "x86_64", InstallKind::Deb).is_err());
        assert!(select_install(&manifest, "linux", "aarch64", InstallKind::Deb).is_err());
        assert!(InstallKind::Deb.installs_in_app() && InstallKind::Deb.needs_password() && !InstallKind::Deb.automatic());
        assert!(!InstallKind::MacApp.needs_password());
        Ok(())
    }

    #[test]
    fn install_selection_matches_platform_architecture_and_container() -> Result<()> {
        let mut manifest = fixture();
        manifest.assets.clear();
        for os in ["linux", "macos", "windows"] { for arch in ["x86_64", "aarch64"] {
            for (kind, suffix) in [("binary", ".bin"), ("appimage", ".AppImage"), ("app", ".app.zip"), ("app", ".dmg")] {
                manifest.assets.push(Asset { name: format!("one-mk-{os}-{arch}{suffix}"), size: 1,
                    sha256: "ab".repeat(32), os: os.into(), arch: arch.into(), kind: kind.into() });
            }
        }}
        for os in ["linux", "macos", "windows"] { for arch in ["x86_64", "aarch64"] {
            for (kind, suffix) in [(InstallKind::Portable, ".bin"), (InstallKind::AppImage, ".AppImage"), (InstallKind::MacApp, ".app.zip")] {
                if (kind == InstallKind::MacApp && os != "macos") || (kind == InstallKind::AppImage && os != "linux") {
                    assert!(select_install(&manifest, os, arch, kind).is_err());
                    continue;
                }
                let selected = select_install(&manifest, os, arch, kind)?;
                assert_eq!(selected.name, format!("one-mk-{os}-{arch}{suffix}"));
            }
            for kind in [InstallKind::Homebrew, InstallKind::Flatpak, InstallKind::WindowsInstaller, InstallKind::Managed] {
                assert!(select_install(&manifest, os, arch, kind).is_err());
            }
        }}
        assert!(select_install(&manifest, "linux", "riscv64", InstallKind::Portable).is_err());
        let duplicate = select_install(&manifest, "macos", "aarch64", InstallKind::MacApp)?.clone();
        manifest.assets.push(duplicate);
        assert!(select_install(&manifest, "macos", "aarch64", InstallKind::MacApp).is_err());
        Ok(())
    }

    #[test]
    fn malformed_crypto_inputs_and_parser_mutations_do_not_panic() -> Result<()> {
        for length in 0..100 { assert_eq!(signature_inputs(&[0; 32], &vec![0; length], b"x").is_ok(), length == 64); }
        let original = serde_json::to_vec(&fixture())?;
        for index in 0..original.len() {
            for byte in [0, b'"', b'{', b']', 255] {
                let mut changed = original.clone();
                changed[index] = byte;
                let _ = parse(&changed);
            }
            let _ = parse(&original[..index]);
        }
        for major in 0..10 { for minor in 0..20 {
            assert_eq!(compare(&format!("{major}.{minor}.9"), &format!("{major}.{minor}.10"))?, Ordering::Less);
        }}
        for remembered in 1..16 {
            let state = State { sequence: remembered, digest: "same".into(), ..Default::default() };
            for offered in 1..16 {
                let mut manifest = fixture();
                manifest.sequence = offered;
                assert_eq!(accept(&manifest, "0.1.12", &state, "same", 0).is_ok(), offered >= remembered);
            }
        }
        Ok(())
    }
}
