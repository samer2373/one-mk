# Signed updates

## Owner setup — configured

The production Ed25519 key is compiled into `update_key.rs`, with
`PRODUCTION_KEY = true`:

```text
1c96a60f7411b89048a7c8e733f1abcf3b21d459f507091ccef753f3fc88bfeb
```

The private key stays on the owner's machine under `~/.config/one-mk-release/`
and in the GitHub Actions secret `ONE_MK_UPDATE_SIGNING_KEY`. Local builds,
tests, checks and installations never need it. Never fetch it for development,
commit it, or paste it into chat. Keep an offline backup on the owner's side.

Tag pushes run `.github/workflows/release.yml`. After staging all release
assets, the publish job signs the manifest's exact bytes, verifies against the
compiled public key, uploads a draft, verifies the asset list, then publishes.
Missing production signing credentials fail publication. Never rebuild or
overwrite a published release. The sequence is `GITHUB_RUN_NUMBER`, also
compiled into release clients as a sequence floor; keep numbering monotonic.

Key rotation requires a bridge release embedding the new key in
`ADDITIONAL_PUBLIC_KEYS` while still signing with the old key. Only then switch
the primary key and secret. `next_key` is advisory, never automatically trusted.
A compile-time guard refuses production with the RFC 8032 public test key.

## Window experience

The default is **check and ask**: Check for updates is on, Update automatically
is off. The notice says **one·mk VERSION is ready**, with **Update now**,
download percentages, verification/install stages, then **Restarting…**. Advanced
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
| Debian package | “Updates come with your package manager” only if `dpkg-query --search` confirms ownership of the daemon path. No package files changed. |
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
