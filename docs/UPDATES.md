# Signed updates

## Publisher setup (maintainers)

The production Ed25519 key is compiled into `update_key.rs`, with
`PRODUCTION_KEY = true`:

```text
1c96a60f7411b89048a7c8e733f1abcf3b21d459f507091ccef753f3fc88bfeb
```

The private key stays on the maintainer's machine under `~/.config/one-mk-release/`
and in the GitHub Actions secret `ONE_MK_UPDATE_SIGNING_KEY`. Local builds,
tests, checks and installations never need it. Never fetch it for development,
commit it, or paste it into chat. Keep an offline backup.

Tag pushes run `.github/workflows/release.yml`. After staging all release
assets, the publish job signs the manifest's exact bytes, verifies against the
compiled public key, uploads a draft, verifies the asset list, then publishes.
Missing production signing credentials fail publication, and so do missing
macOS signing/notarization secrets (`MACOS_CERT_P12`, `MACOS_CERT_PASSWORD`,
`NOTARY_APPLE_ID`, `NOTARY_PASSWORD`, `NOTARY_TEAM_ID`): the Mac updater
refuses an ad-hoc signed app. Publishing also waits for the workspace tests
and clippy on all three systems and the release checks. Never rebuild or
overwrite a published release. The sequence is `GITHUB_RUN_NUMBER`, also
compiled into release clients as a sequence floor; keep numbering monotonic.
`install.sh` carries its own floor, `MIN_SEQUENCE`, for fresh installs; raise it with each release to the previous release's sequence, which CI checks on pull requests (docs/PACKAGING.md, "install.sh and replayed releases").

A bad release is withdrawn by publishing a newer one with the repository
variable `ONE_MK_REVOKED` (space-separated versions no updater may install
any more) and, when old installations must not update in place,
`ONE_MK_MINIMUM_SUPPORTED_VERSION`; `scripts/make-update-manifest.sh` puts
both into the signed manifest (empty: none revoked, minimum 0.1.0).

Key rotation requires a bridge release embedding the new key in
`ADDITIONAL_PUBLIC_KEYS` while still signing with the old key. Only then switch
the primary key and secret. `next_key` is advisory, never automatically trusted.
A compile-time guard refuses production with the RFC 8032 public test key.

## Window experience

The default is **check and ask**: Check for updates is on, Update automatically
is off. The notice says **oneMK VERSION is ready**, with **Update now**,
download percentages, verification/install stages, then **Restarting…**. **Settings**
also exposes both switches. Automatic installation is opt-in and happens only
while the window is open. Users need no terminal or configuration editor.

The daemon performs one daily-due check at startup. The window requests a
due-only check on opening and once per day through its existing status watcher.
They share a lock and persisted attempt time, saved before network I/O. Closing
the window adds no updater polling loop; disabled checks fetch nothing.
Health reads cached information only: **Up to date (VERSION)**,
**Update available — Update now** (action `update-now`), or
**Couldn't check: REASON**. Opening Health does not fetch a release.

After installation, the window stops sharing, starts the replacement daemon,
waits for its local control channel, opens the replacement window, and closes
itself. AppImage restart uses the original image, not its old temporary mount.
The tray can remain mapped from the previous build until it exits; its files
are updated in a bundle/AppImage for the next launch.

## macOS app flow

Select the running architecture's signed `kind: app` asset ending in `.app.zip`.
The accompanying `.dmg` is not ambiguous: select the ZIP, never the disk image
or a raw daemon. Existing packaging publishes both architectures' ZIPs,
containing `one-mk.app` with window, daemon, tray, and resources.

1. Locate the running `.app` ancestor, wherever installed. Reject
   AppTranslocation and `/Volumes/` locations.
2. Create exclusive staging beside the installed bundle. If the account cannot
   change the parent folder, explain that plainly and offer **Download** (opens
   the release page). Never elevate or bypass verification.
3. Download only the asset named by the verified manifest; verify exact size
   and SHA-256 before extraction with the system `ditto`.
4. Run `/usr/bin/codesign --verify --deep --strict` on running and candidate
   bundles. Read `TeamIdentifier` using `--display --verbose=4`; both must have
   a real ten-character team ID and match. Unsigned/ad-hoc, invalid, or
   differently signed apps are refused. Probe the candidate daemon version
   with a five-second timeout.
5. Persist a pending receipt and stop sharing. Rename the old bundle to hidden
   sibling `.NAME.previous.app`, then rename the staged bundle into place on
   the same filesystem. Restore the old bundle if the second rename fails.
   Each rename is atomic, but the pair is **not crash-atomic**; a crash in the
   gap can require recovery from the retained sibling.
6. Relaunch through the window. Keep the previous bundle until the new window's
   first frame acknowledges its own compiled version. Old/mismatched windows
   cannot acknowledge. Then remove backup and receipt. Another update cannot
   overwrite a pending backup. If restart fails, the surviving window offers
   **Restore previous version**; rollback verifies the team again and preserves
   signed-release replay protection.

An app running from a DMG or translocated folder offers **Move to Applications**.
Copy with `ditto` into staging in `/Applications`, verify both identities, rename
into `/Applications/one-mk.app`, then relaunch. Never overwrite an existing copy.
If permissions prevent moving, explain the Finder drag-and-drop alternative.

## Installation kinds

| Kind | Behavior |
| --- | --- |
| Linux AppImage in the user's home | Verify the complete signed image's size/hash/version, retain the previous image, replace and restart from the original file. Candidate execution requires working runtime/FUSE support. |
| Linux/macOS portable `one-mk` in the user's home | Signed raw binary replacement with `one-mk.old`; restart sharing and window. Separate window/tray binaries are **not** upgraded. |
| Windows portable `one-mk.exe` in the user's home | Signed raw executable replacement; rename the mapped executable aside, restore if installing fails. Separate window/tray binaries are not upgraded; no elevation. |
| Windows installed app | Download fallback for the existing per-user installer. Silent installer/whole-directory replacement is **not implemented**. Program Files/WindowsApps are not modified. |
| Debian package | Installed from the window too, with one system password prompt (see below), only if the package's file list confirms ownership of the daemon path. |
| Homebrew / Flatpak | Package-manager message; no direct replacement. |
| Unknown/system-wide portable path or AppImage outside home | Download fallback; do not guess ownership or modify the installation. |

Portable updates use same-directory temporary files, signed length/SHA-256,
executable permissions and bounded version probes before replacement. Unix
file replacement is atomic. Windows has a rename-aside gap, not crash-atomic.
Root/elevated operations and symlink targets are refused. Backups never reset
release high-water marks. The advanced CLI still provides check, rollback and
preference controls, but window users need none of them.

## Trust and transport

The embedded Ed25519 key is the trust anchor, not GitHub, HTTPS, a mirror, or
`SHA256SUMS`. Persisted locked high-water marks reject older sequences,
same-sequence equivocation, version downgrades, revoked targets and unsupported
minimum versions. Identical repeated checks are allowed. Deleting state loses
learned replay protection. No release-time expiry penalizes offline users.

### Freshness for `install.sh`

A fresh install has no stored sequence, and a signed list never expires, so
without more, whoever serves the download could pin a new computer to an older
signed release. A static signed list cannot prove it is the latest: that needs
a clock, or something signed more recently. What is done instead:

- The update list has **no expiry or freshness field and cannot get one**:
  the parser above refuses unknown fields and every installed client has it.
  `make-update-manifest.sh` output is unchanged, so every updater in the field
  accepts it as before (pinned by `scripts/test-update-manifest.py` and a test
  in `crates/ctl/src/update_logic.rs`). The updater itself needs no expiry: its
  high-water marks already reject anything older than what it has seen.
- `install.sh` carries `MIN_SEQUENCE` (raised in each release's pull request to
  the previous release's sequence, enforced in CI by
  `check-release-consistency.py --base`), refuses an older version over a newer
  installed one, prints the verified version, number and date, notes a release
  older than 180 days, and, if asked with `ONE_MK_MAX_AGE_DAYS=N`, refuses one
  older than N days by the computer's clock. Without that variable an old
  release is never refused for its age, so the newest release stays
  installable however quiet the project is.
- To withdraw a release for fresh installs at once, raise `MIN_SEQUENCE` in a
  pull request (to at most the newest published sequence).

Remaining, and inherent: a fresh install can be handed the release just before
the newest (and nothing older); an attacker who replaces `install.sh` as well,
or a computer without OpenSSL 3 (macOS), is not covered; the installed oneMK's
first update check offers the newest release. A periodically re-signed
freshness statement would close more, at the price of the signing key in a
scheduled job; it is not implemented (docs/PACKAGING.md, "install.sh and
replayed releases").

The manifest is at most 256 KiB; its detached signature is exactly 64 bytes.
Strict JSON parsing refuses duplicate/unknown fields, unsafe names, invalid
dates/hashes/versions and ambiguous selections. Assets are limited to 512 MiB.
Transport uses rustls/webpki hostname validation, bounded HTTP, timeouts and at
most three redirects, restricted to the configured origin and explicit GitHub
release hosts. No credentials, fragments, HTTPS downgrade or test-key switch
is allowed in the shipped CLI. Loopback HTTP is compiled only into unit tests.

Non-goals: compromised signing key, malicious authenticated publisher, hostile
local account access, replacing the entire settings directory, and package
manager trust. Code-signature checks do not add an online notarization lookup.

## Validation and Mac acceptance

Tests cover OS/arch/container selection (ZIP versus DMG), hash/signature/size
corruption, wrong teams and signature failures through a fake runner, swap
restoration through a fake filesystem, real temporary-directory swaps,
first-start backup retention, and translocation detection.

Explicitly run `update::tests::python_http_portable_end_to_end` with `--ignored`:
it starts `python3 -m http.server` on 127.0.0.1, serves a temporary fake 0.1.20
release signed by the RFC 8032 TEST key, updates a temporary 0.1.19 binary,
starts it and rolls back while retaining replay state. The explicit test-only
key path never weakens production verification; the test also asserts the
production key rejects its signature. No private production key is used.

Linux tests/cross-compilation are **not real macOS or Windows validation**.
AppImage/FUSE, code-signature tools, permissions, App Translocation, mapped
Windows executables and graphical restart need real platform acceptance.
See the implementation report for actual check outcomes.

On a disposable Mac account with an older production-signed release:

1. Applications and writable custom folder: banner, Update now, progress,
   restart, new version, resumed sharing, backup removed after new window starts.
   Repeat on Apple silicon and Intel.
2. Offline, corrupt download and wrong-team/unsigned controlled fixtures:
   refuse without replacing the runnable app.
3. Unwritable parent: plain permission text and Download. DMG/translocation:
   Move to Applications or Finder explanation; never overwrite an existing copy.
4. Force second rename failure: old app restored. Block new launch: backup
   remains, Restore previous version works, old window cannot acknowledge it.
5. Default asks before installing; opt-in installs only with the window open.
   Close it and verify no extra updater timer. Check narrow/HiDPI and VoiceOver.

## Debian package: install from the window

**Check for updates** (Settings > Updates) asks the release server now; when a
newer version is known the same button reads **Update to VERSION**. For a
package install the update then:

1. downloads the `.deb` named in the signed update list, into a private
   folder (`update-staging`, mode 0700, in the settings folder) as the person,
   and checks its size and SHA-256 against that list;
2. checks with `dpkg-deb --field` that it is the `one-mk` package at the
   signed version, and puts the signed update list and its signature
   (`update-manifest.json`, `update-manifest.json.sig`, the exact downloaded
   bytes) beside the package, under the name the list gives it;
3. asks the system for the password once (`pkexec`, the desktop's own dialog,
   with the polkit action `io.github.onemk.OneMk.update` the package ships in
   `/usr/share/polkit-1/actions/`: "Authentication is required to install the
   signed oneMK update", asked every time) to run
   `/usr/bin/one-mk update --install-deb FOLDER` as root;
4. restarts the service and the window like every other update.

The root step believes nothing the asking account says: its only input is
the staging folder. It

- opens the folder without following links and requires that it is named
  `update-staging`, owned by the account pkexec names (`PKEXEC_UID`) and not
  writable by anyone else; every file in it is opened relative to that handle,
  without following links, and must be a regular file that account owns
  (a link, pipe or device is refused);
- verifies the signature of the update list against the key compiled into
  the installed `/usr/bin/one-mk` (never a key from the folder);
- refuses a list that is not newer than the installed version, is older than
  the build's release sequence or than a list the root step accepted before
  (its own record in `/var/lib/one-mk/update-state.json`, root only), reuses
  a sequence with other contents, is revoked, or needs a newer installed
  version;
- copies the `.deb` the list names through the one handle it opened into a
  root-only file in `/var/lib/one-mk`, hashing while it copies, and requires
  the signed size and SHA-256;
- checks the package name and version of that copy with `dpkg-deb`, and runs
  `dpkg -i` (with a clean environment) on that same copy, then deletes it.

Replacing or relinking the staged file after it was opened changes nothing.
It never installs without the person pressing the button: **Install them
without asking** stays off for package installs, because a password is needed.
Cancelling the prompt changes nothing. The hidden `--install-deb` option does
nothing when run by hand (it refuses unless started as root through `pkexec`).
Without `pkexec` the update says so and the package can be installed by hand.
Tests (`update::tests::the_root_step_*`, `a_package_swapped_after_the_check_*`)
cover tampering, foreign keys, downgrades, wrong hashes and sizes, a swapped
or linked file, a pipe, replayed older lists, foreign owners and a shared
folder; the real `pkexec` dialog and `dpkg -i` need a real desktop to check.
