# Signed updates (5G)

## Security design

The trust anchor is the Ed25519 public key compiled into `update_key.rs`, NOT
GitHub, SHA256SUMS, HTTPS alone, or a mirror. The checked-in RFC 8032 test key is
**public test material, not a production key**. Production update checking and
applying remain disabled until the owner replaces it and sets `PRODUCTION_KEY`.
No background apply, elevation, restart, notification or periodic timer exists.

Threats: a compromised GitHub account/asset, network attacker or malicious mirror
cannot forge a manifest; signatures cover the exact JSON bytes. Hash and exact
length checks detect corrupted, substituted and truncated assets. A persistent,
locked high-water mark rejects older sequences, same-sequence equivocation and
version downgrade, including after a rollback. A repeated identical manifest is
allowed so checking does not prevent applying. First contact cannot distinguish
an old signed release newer than the installed version from a current release;
the compiled release sequence floor mitigates that bootstrap replay window.
Deleting local state removes learned replay protection. Clock rollback does not
reset the high-water mark. No release-time expiry: offline users can update.

The signed `update-manifest.json` is at most 256 KiB. Schema 1 fields: `version`
(SemVer without `v`), `release_time` (UTC RFC3339), `sequence` (positive integer),
`minimum_supported_version`, `assets` (name, size, sha256, os, arch, kind), optional
`revoked` versions and `next_key` (64 lowercase hex characters). The detached
`update-manifest.json.sig` is exactly 64 raw Ed25519 signature bytes. Duplicate
fields, unknown fields, unsupported platforms/kinds, path-like names, invalid
hashes, invalid versions and ambiguous asset selections are rejected. Only
assets named in the signed manifest are downloaded. Their addresses come from
the unsigned release listing, so each is restricted to the configured host or
GitHub's release hosts, and the signed size and SHA-256 are checked before
anything is installed. Revocation blocks installing a revoked target; a revoked installed
version may still upgrade. Minimum version requires manual installation.

Non-goals: compromised signing key, malicious signed executable, a hostile local
user with access to this account, rollback of the whole config directory, package
manager trust, platform notarization and updating companion app/tray executables.
The updater does not sandbox executing a validly signed candidate's `--version`.

Key rotation: publish a manifest with `next_key` under the current key, then ship
a bridge release embedding that new key in `ADDITIONAL_PUBLIC_KEYS` while keeping
`PUBLIC_KEY` and the signing secret unchanged. Only switch `PUBLIC_KEY` and the
release signing secret after users have installed that bridge release. Remove
the old key in the following release. `next_key` is advisory, never silently trusted from disk; clients
that miss the bridge must reinstall manually. A compromised key requires manual
reinstallation from an independently authenticated source.

## Owner setup (before shipping)

1. Run `bash scripts/update-keygen.sh /secure/offline/one-mk-update` locally.
   OpenSSL creates an unencrypted PKCS#8 PEM private key with mode 0600. Keep an
   offline backup; never commit it or paste it into chat. The script prints the
   32 public bytes as a Rust array and hex.
2. Replace `PUBLIC_KEY` in `crates/one-mk/src/update_key.rs` with those bytes and
   set `PRODUCTION_KEY = true`. Commit the public key only. Rebuild all platforms.
3. In GitHub repository Settings → Secrets and variables → Actions → New repository
   secret, name it `ONE_MK_UPDATE_SIGNING_KEY`, value the entire private PEM.
   Only the tag-triggered publish job receives this secret; no PR job does.
4. Tag a version matching `scripts/version.sh`. The draft release is populated,
   its manifest signed, verified against the committed public key, checksummed,
   uploaded and only then published. Without the secret, release publication
   remains possible but explicitly unsigned; this updater refuses it.
5. Run `one-mk update --check` on that release before announcing auto-update.

The release sequence is the GitHub workflow run number (monotonically increasing
for this workflow). Re-runs of one run have the same sequence: do not change
signed bytes or rebuild published releases on a re-run. Never reset/recreate the
workflow numbering; choose a higher sequence if migrating workflows. Build
sequence floors are embedded from `GITHUB_RUN_NUMBER`.

## Client behavior

`one-mk update --check` checks without fetching an executable. `one-mk update`
explicitly applies a newer verified release. `one-mk update --rollback` restores
the previous portable binary without clearing replay protection. Configure
`[update] check = false` in settings.toml or `one-mk update --check-on-start off`.
`[update] url` / `--url` overrides the GitHub-compatible releases/latest endpoint,
not the signing key. Only HTTPS is allowed in production. HTTP on literal
127.0.0.1 is allowed only in unit-test builds, never the shipped CLI.

Startup checking is one worker at startup, at most once per 24 hours, with the
attempt persisted before I/O. Failures go to debug logs only. It never applies.
`status` reports cached availability without network access. Unknown installation
paths are managed: the updater will not guess that an executable is safe to
replace. Known managed paths include /usr, /opt, Homebrew, Flatpak, macOS .app
bundles and Windows Program Files/WindowsApps. Portable updates require the
normal per-user `~/.local/bin/one-mk`, a Windows per-user portable directory, or
an AppImage identified by its original APPIMAGE path. Package-managed installs
print a manual command and are never modified. The per-user Windows installer
path (`AppData/Local/Programs/one-mk`) and Linuxbrew are also recognized as managed.
All explicit update operations refuse root/elevation. Symlink targets are refused.

Portable tarball installs update only the CLI/daemon using a separately signed
raw binary asset. The window and tray still require manual installation of the
complete release. AppImage replacement verifies the complete candidate using
its `--version` interface before replacement; filesystem/FUSE support is needed.
The candidate is streamed into a unique file in the target directory, length
and SHA256 checked, executable permissions set, and `--version` checked with a
timeout before swapping. A previous copy is saved as `one-mk.old`. Unix rename
is atomic; Windows renames the running binary aside before the final rename and
restores it on failure. No restart is automatic: stop and run again yourself.

Transport uses rustls with webpki roots, hostname validation and no custom trust
bypass, bounded headers/body, timeouts and at most three redirects. Redirects
are limited to the original release host and explicit GitHub asset hosts; no
credentials, userinfo, fragments or HTTPS downgrade. TLS roots authenticate the
server, not a pinned GitHub leaf certificate. Dependencies reuse rustls/ring/url/
semver already in the tree; serde_json adds strict typed bounded JSON parsing,
webpki-roots supplies public CA anchors. No OpenSSL runtime dependency. OpenSSL
is only the owner/CI signing tool.

## Troubleshooting and platform validation

Unsigned/untrusted release: install manually or ask the publisher to complete
key setup; never bypass signature verification. Bad hash/size: retry after the
publisher repairs the release. Replay: do not delete state to bypass it; publish
a higher signed sequence. Permission failure: use the package manager, not sudo
with the updater. Interrupted Unix replacement leaves either the old or verified
new executable; use `--rollback` if needed. Disk/file permission errors are fatal.

Windows/macOS executable replacement is **untested on real operating systems**;
cross-compilation is not runtime validation. Linux desktop/window and hardware
validation, and exact local validation results, are recorded in the implementation
report. Never assume passing pure tests proves platform-specific replacement.

Manual hardware checks: disable startup checks first; use a copied portable
install and a throwaway `--home`. Check unsigned, valid and corrupt local release
fixtures in the test harness; ensure the original executable is untouched. With
a production signed release, check, apply, restart, check again, rollback and
check again; sequence protection must persist. Repeat on Windows with the old
exe running and on macOS outside .app bundles. For managed installs ensure only
instructions appear. Interrupt download and replacement, restart and verify
either the original or authenticated candidate remains runnable. Confirm no
network requests or wakeups after the startup worker finishes.

## Local validation record (2026-10-30)

All final test launches use an empty environment with only PATH, throwaway HOME,
mode-0700 XDG_RUNTIME_DIR and ONE_MK_HOME. Cargo additionally receives its cached
CARGO_HOME/RUSTUP_HOME; private Xvfb additionally provides only its own DISPLAY
and XAUTHORITY. No live Wayland, session D-Bus or notification environment is
passed. Tests neither install programs nor contact real release servers.

- `cargo test -j 2 --offline --workspace` **is not green** in this isolated
  environment: the unchanged input crate's
  `portal::tests::the_portal_going_away_lets_go_and_reconnects_when_it_is_back`
  times out waiting for its fake portal to reconnect, including when retried
  alone. This is outside 5G; no input-backend code was changed to hide it.
  With only that test explicitly excluded, 758 tests pass, seven are already
  ignored. Keep this failure a release gate, not a claimed successful full run.
- Linux workspace/all-target clippy and the requested Windows/macOS
  input/proto/ctl/app/tray all-target clippy matrices pass with `-D warnings`.
  The main executable's Windows/macOS OS glue has **not** been cross-checked
  (ring requires the corresponding C toolchain), and neither OS was run.
- Local signed HTTP fixtures cover signature/key/manifest mutation, malformed
  HTTP framing, bounded parser mutation/truncation, sequence decision properties,
  exact size/hash, interrupted transfer, killed download child, replacement,
  failed/hung version execution (including Unix process-group cleanup), rollback,
  stale-temp recovery and forbidden redirects.
  Temporarily bypassing the hash guard makes the bad-hash assertion fail;
  the guard is restored and the unchanged regression test passes afterward.
- `scripts/check-updates.sh` parses YAML, checks all 13 shell run blocks and
  exercises manifest generation, ephemeral signing, embedded-key verification,
  checksums, unsigned warnings, tamper refusal and draft/upload/publish ordering
  using a stub `gh`. **GitHub Actions and real production publication did not run.**
- `scripts/check-update-runtime.py target/debug/one-mk` runs the real CLI,
  explicit-target portable rollback, settings toggles/errors, two isolated null
  daemons, kill-9/restart/status/stop, and legacy-format settings preservation.
  It never invokes or replaces an installed binary. The legacy fixtures are
  handwritten in the previous schema, **not generated by a prior build**.
- Real egui windows were captured and visually inspected on private Xvfb:
  no update, available, real Update error, keyboard Tab/Enter, 760px width,
  long-version truncation, 2x scaling and a disabled loading button. Loading
  uses a delayed shell fixture; availability uses a local cached-state fixture.
  The release-notes link was not opened (no external browser/network test).
  Screen-reader behavior, many-peer layouts and a real successful UI update
  have not been validated. The banner adds no animation or polling timer.
- Real null-daemon idle samples: checks ON **0 CPU ticks / 5 seconds,
  31,588 kB RSS**; OFF **0 ticks / 5 seconds, 31,236 kB RSS**, at 100 ticks/second.
  These are short debug-build observations, not a universal zero-CPU guarantee.
  The placeholder-key gate disables the startup network worker, so production
  worker cost and post-request idle behavior remain unmeasured. The window
  already polls at 250ms; 5G adds one file stat to that existing poll, no timer.

Logs and screenshots are local ignored files under `target/update-validation/`.
A final read-only host process audit found **zero** remaining worktree test
executables, private windows or throwaway-home processes; owner processes were
not signaled or replaced.
The full doctor command was not launched because its unrelated existing probes
can multicast/query desktop services; its update hook is compiled and reviewed.
Physical sleep, real peer disconnect/reconnect, Bluetooth, wlroots and real input
backends were not exercised. This implementation is **gated, not certified for
production rollout** until owner key setup, the unrelated test failure and the
real-platform checks are resolved.

## Manual release acceptance script

Use dedicated test accounts/machines, never an owner's active installation.
Complete owner signing setup first. Do not enable the public test key. The
shipped CLI accepts HTTPS only; loopback HTTP fixtures run inside Rust tests.

1. Install an authenticated older build into a **copied** portable test home
   (`$TEST_HOME/.local/bin/one-mk`, or the documented Windows portable directory).
   Set HOME, XDG_RUNTIME_DIR (0700), ONE_MK_HOME to test directories with an empty
   environment. Never inherit WAYLAND_DISPLAY, DBUS_SESSION_BUS_ADDRESS or
   XDG_SESSION_TYPE. On Windows/macOS use a dedicated test account/session.
2. Run that exact copied executable with `--home "$TEST_HOME/config" update
   --check`. Expect a newer signed version and unchanged executable bytes. Run
   `update`, then `--version`; expect the new version, `one-mk.old` and a manual
   restart hint. Companion window/tray installation remains manual.
3. Start only a null daemon with `--home "$TEST_HOME/config" run --backend null
   --no-discovery --no-notify --no-clipboard --port <free>`. Run the copied CLI's
   `stop`, then start again with those same flags. Expect a clean stop/start;
   inspect `status` without a network request. Never use the bare restart hint
   during this isolated test, because it omits these test safety flags.
4. Run `update --rollback` and `--version`; expect the previous version while
   the sequence high-water mark remains. Check again; the same authenticated
   latest release remains eligible, an older signed sequence must be rejected.
   On Windows repeat while the old executable is running. If the process dies
   in Windows' rename-aside gap and the executable is missing, recover by
   manually renaming `one-mk.old` back to `one-mk.exe`; this gap is not atomic.
5. In a controlled signing fixture, corrupt/truncate the payload, signature and
   manifest, interrupt a download and kill the updater. Expect failure and the
   original executable intact; the next attempt cleans reserved temporary files.
   Repeat on real AppImage/FUSE, Windows and macOS hardware before rollout.
6. On disposable managed installations run `update`; expect instructions only
   (apt, brew, Flatpak, Windows installer or macOS .dmg), no changed installed
   files, no elevation. Test sleep/offline recovery and copy **actual previous
   build** settings/devices into the test home to finish migration acceptance.
7. After the startup worker exits, measure CPU/wakeups/RSS with checks on/off
   for at least 60 seconds. Expect no updater timer/network traffic. Exercise
   banner keyboard/screen-reader access at narrow/HiDPI sizes. Stop all test
   processes and confirm none remains before removing test directories.
