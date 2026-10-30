# Hostile review: wave5 5G (signed update), 5H (local remap groundwork), 5E (keyboard layout foundations)

Reviewer: Claude (read-only on worktree `wave5`). All experiments ran in /tmp copies with a clean env and the shared target dir.
Evidence: probe crate `/tmp/rv5/probe` (against the real ctl/proto sources), mutated workspace copy `/tmp/rv5/ws`.

## Ranked findings

### 1. HIGH: `one-mk update` can replace an unrelated AppImage because it trusts the inherited `APPIMAGE` env var
- Where: `crates/one-mk/src/update.rs:350-356` (`target()`), `crates/ctl/src/update_logic.rs:174-176`.
- `target()` takes `std::env::var_os("APPIMAGE")` as the update target if it is set. Nothing checks that this process actually runs from that AppImage. The AppImage runtime exports `APPIMAGE`/`APPDIR`/`OWD` and child processes inherit them. This is a well-known leak: any terminal opened inside an AppImage'd IDE or terminal emulator has `APPIMAGE` pointing at that app.
- Repro: the user has the portable `~/.local/bin/one-mk`. They run `one-mk update` from a terminal inside e.g. `~/Apps/Code.AppImage` (`APPIMAGE=/home/me/Apps/Code.AppImage`). `install_kind("/home/me/Apps/Code.AppImage", "/home/me", true, false)` returns `InstallKind::AppImage` (confirmed by `/tmp/rv5/probe` test `foreign_appimage_from_leaked_env_is_update_target`). `apply()` then downloads one-mk's AppImage, `version_check` passes (the *candidate* is one-mk), and `replace()` renames it over `Code.AppImage`. The IDE ends up as `~/Apps/one-mk.old`. The same path also misclassifies the real portable binary, so it can never update from that shell. `doctor()` has the same wrong classification.
- Fix: honour `APPIMAGE` only when `current_exe()` is inside `$APPDIR` (or `/proc/self/exe` is under the AppImage mount). Before swapping, run the *existing target* with `--version` and require a `one-mk ` prefix. Refuse when the target's file name does not start with `one-mk`.

### 2. MEDIUM: an AppImage renamed to `~/.local/bin/one-mk` gets overwritten by the raw CLI binary
- Where: `crates/ctl/src/update_logic.rs:175-176`.
- The kind is AppImage only if the path ends in `.AppImage`. Otherwise an exact match on `{home}/.local/bin/one-mk` gives Portable, even when `appimage == true`. Users commonly rename an AppImage into `~/.local/bin` under the command name.
- Repro: `install_kind("/home/me/.local/bin/one-mk", "/home/me", true, false) == Portable` (confirmed: probe test `renamed_appimage_in_local_bin_is_treated_as_portable_binary`). `apply()` then selects kind `binary` and replaces the full AppImage (window, tray and libraries) with the bare CLI/daemon `.bin`.
- Fix: if a verified AppImage (see finding 1) is running, the kind is always AppImage, whatever the extension.

### 3. MEDIUM: production gating is one boolean; flipping `PRODUCTION_KEY` with the RFC 8032 test key left in place ships a forgeable updater
- Where: `crates/one-mk/src/update_key.rs:2-9`, `scripts/sign-update-manifest.sh`, `.github/workflows/release.yml:138-143`.
- `PUBLIC_KEY` is RFC 8032 TEST 1. Its private seed is public and is even checked in at `update_tests.rs:56`. Setting `PRODUCTION_KEY = true` without replacing `PUBLIC_KEY` compiles and passes CI. If the signing secret is absent, the sign script prints "UNSIGNED release" and exits 0, so the release still publishes binaries that trust a world-known key. From then on, anyone who can serve the manifest can sign an update those binaries accept: a compromised GitHub account or asset, a mirror, or a `--url`/`[update] url` override. That is exactly the threat the design claims to stop.
- Fix: add `const _: () = assert!(!PRODUCTION_KEY || !matches!(PUBLIC_KEY, [0xd7, 0x5a, 0x98, 0x01, ..]));` (compare all 32 bytes) and the same check in CI. In CI, fail the release when `PRODUCTION_KEY = true` and the secret is missing.

### 4. MEDIUM (Windows): rollback, or a second update, before restarting fails, because `one-mk.old` is the running daemon image
- Where: `crates/one-mk/src/update.rs:446`.
- On Windows `replace()` does `remove_file(one-mk.old)` and then renames the target aside. After the first update the still-running daemon (restart is manual) is mapped from `one-mk.old`. Deleting a mapped executable fails with ERROR_ACCESS_DENIED. So the most likely rollback scenario fails with an opaque I/O error: the new build is bad and the user rolls back right away. A second update before restart fails the same way.
- Fix: rename the existing `one-mk.old` to a unique `.one-mk-update-stale-*.exe` (renaming a running image is allowed) and delete stale files later. Alternatively use `MOVEFILE_DELAY_UNTIL_REBOOT`.

### 5. LOW-MEDIUM: a rejected check leaves a stale `available` in state, so the banner keeps advertising a revoked release
- Where: `crates/one-mk/src/update.rs:341-346`; `cached_version`/`notice` read `state.available`.
- `accept()` errors on revoked, minimum-version and downgrade cases, and `checked()` returns before updating state. If 0.1.13 was cached as available and is then revoked, the window, `status` and `manage` keep saying "0.1.13 is available; run one-mk update", and every attempt fails. The minimum-version case never sets `available`, so users who need a manual reinstall see nothing at startup (failures go to debug logs only).
- Repro: probe test `revoked_release_leaves_previous_available_untouched` (`accept` errs, so the caller never clears `available`).
- Fix: on any signed-but-rejected manifest, clear or replace `available` (for example store a "manual install required" marker) and save.

### 6. LOW-MEDIUM: rollback lands on a binary that cannot read the new settings
- Where: `crates/one-mk/src/tuning.rs` Deserialize (5H removed the `"a local rule can't be for one device"` error that main still has); `crates/one-mk/src/update.rs:41` (`[update]` uses `deny_unknown_fields`, the only such section in settings).
- After `one-mk keymap add X Y --local --device NAME`, a pre-5H binary fails `toml::from_str` for all of `settings.toml`. That is exactly what `update --rollback` restores. `load_or_default` then silently runs with default settings. Separately, a future version adding any `[update]` key makes *this* version reject the whole file after a rollback. A typo in `[update]` today discards every setting.
- Fix: drop `deny_unknown_fields` on `update::Settings`, as every other section does. Document or gate `--local --device` against rollback, or validate settings before rollback and warn.

### 7. LOW: several guards in the update download are not covered by any test
- Mutation run in `/tmp/rv5/ws`. I removed three guards at once and all 9 `update::tests` still pass:
  - (a) `allowed()` forced to `true`;
  - (b) `ensure!(size == asset.size, ...)` deleted;
  - (c) both `regular(target)` symlink checks (apply and replace) deleted.
- Why: (a) the redirect tests target `https://attacker.invalid`, which fails at DNS anyway. (b) the size test sets `size -= 1`, which trips the Content-Length limit first, and the hash catches truncation. (c) no symlink test exists. There is also no test for the tag/version mismatch at `update.rs:339`.
- Fix:
  - Redirect to `http://127.0.0.1:<second server port>` that serves a valid body and assert refusal.
  - Serve a shorter body with no Content-Length and assert the size error message specifically.
  - Add a symlink-target test.

### 8. LOW: one 120 s deadline covers the whole download (up to 512 MiB)
- Where: `crates/one-mk/src/update.rs:223`.
- Connections below roughly (asset size / 120 s) can never update, and the AppImage is tens of MB. The download is not resumable.
- Fix: use a stall timeout (no progress for N s) plus a size-proportional cap.

### 9. LOW: no macOS download hint, because .dmg and .app.zip make the `app` selection ambiguous
- Where: `scripts/make-update-manifest.sh` (both files become `kind=app`); `update.rs:552` swallows the error with `.ok()`.
- Confirmed by probe test `mac_dmg_and_app_zip_make_app_selection_ambiguous`.
- Fix: map `.dmg` to its own kind, or prefer `.dmg`.

### 10. LOW: UPDATES.md says API-supplied download URLs are not trusted, but they are used
- Where: `docs/UPDATES.md` vs `update.rs:316-324`.
- The code looks up assets by the signed name but downloads from the unsigned API `browser_download_url`, restricted only by the host allow-list. Hash and size make this safe, but the doc overclaims.
- Fix: construct the URL as `https://github.com/<repo>/releases/download/<tag>/<signed name>`, or reword the doc.

### 11. LOW (5H): `decide()` lets the shortcut flag override OS ownership on key-up, which can leave keys stuck locally
- Where: `crates/input/src/localmap_handoff.rs:113-115`; the test at line 207 asserts it for every owner and direction.
- An OS-owned down (passed to the OS) whose up is flagged `shortcut` gets `Route::Shortcut` instead of `Pass`. That contradicts the doc matrix ("OS-owned up: Pass to OS"). If native glue flags the up of the chord's modifier keys (Ctrl/Alt/Shift held for a hotkey), the OS never sees those ups.
- `decide()` also returns Suppress for an `Owner::Local` up when `healthy == false`. It relies on the caller having already called `fail()` to release the replacement; the harness does that, but the type does not enforce it.
- Fix: check `!down && owner == Os` (Pass) before the shortcut branch, or have `decide()` require shortcut only on downs. Also fix the test, which mirrors the implementation's match and so cannot catch spec drift.

### 12. LOW (5H): the handoff "gate" cannot be read by a hook as LOCAL-REMAP.md requires
- `Handoff` is a plain `&mut self` struct. The doc requires the hook to read only atomics or snapshots with no mutex, and says starting a transition "closes the local gate immediately". That only holds for whoever owns the struct; there is no atomic publication.
- This is groundwork, but the doc reads as if the fence already exists.

### 13. LOW (5E): XKB reverse lookup returns keypad keys for `*` and `+`
- Where: `crates/input/src/xkb.rs` `reverse()` scans codes 1..=255 with no modifiers first.
- On US, `reverse('*') = code 55` (KP_Multiply) and `reverse('+') = code 78` (KP_Add), instead of Shift+8 and Shift+=. Confirmed in `/tmp/rv5/ws` (`PROBE star=Some(Chord { code: 55, modifiers: [] }) plus=Some(Chord { code: 78, modifiers: [] })`).
- Effect: Ctrl+Plus and similar shortcuts arrive as keypad keys; some apps treat them differently.
- Fix: skip keypad codes (55, 71-83, 96, 98, 117, 121) or try them last.

### 14. LOW (5E design, not wired): Receiver refcounts are separate from the physical Key path
- `typing::Receiver` counts modifiers privately, while physical `Control::Key` goes through `Owner.held`. Once wired, a chord's Shift up can release a Shift the user still physically holds, and vice versa.
- Unify the refcounts with `Owner.held` before wiring.

### 15. LOW (CI hardening)
- The publish job, which receives `ONE_MK_UPDATE_SIGNING_KEY`, runs `actions/checkout@v4` and `actions/download-artifact@v4` by mutable tag; elsewhere the repo pins by SHA.
- The env is step-scoped, but an earlier compromised action could tamper with `scripts/*` or `GITHUB_ENV`/`BASH_ENV` before the signing step. Pin by SHA.

## Verified OK
- Ed25519 is verified over the exact bytes before parsing, and the same bytes are parsed (no TOCTOU). Signature length is exactly 64. The gate (`PRODUCTION_KEY=false`) blocks both the startup worker and `run_inner` before any network I/O.
- HTTP on 127.0.0.1 is allowed only under `cfg!(test)`.
- The manifest parser:
  - is bounded at 256 KiB, with recursion bounded by serde;
  - rejects duplicate keys (serde derive), unknown fields, and path-like/`..`/leading-dot names;
  - checks the hex, calendar dates, asset counts and size limit;
  - refuses ambiguous binary/appimage assets.
- Replay and downgrade:
  - sequence high-water mark, with same-sequence equivocation refused by digest;
  - the floor equals the build's own release run number;
  - `highest_version` survives rollback;
  - the mutation-sensitive policy tests do kill the obvious mutations (floor, digest, highest).
- Download: the hash is computed over exactly the bytes written, compared in constant time and checked against the signed size. Framing is strict (CL+TE refused, duplicate CL refused, gzip refused, chunk bounds). Redirects: at most 3, no https to http downgrade, host allow-list (functionally correct, just untested; see finding 7). Header budget is 32 KiB; no CRLF injection is possible via `Url`.
- Staging uses `create_new` (O_EXCL) in the target dir. Unix replace keeps the old binary until the atomic rename. A killed download leaves the original intact and is cleaned on retry. The version probe uses a process group and is killed on timeout.
- CI: the secret is written via the printf builtin to a 0600 mktemp dir and removed by trap. It is never echoed. The step refuses to sign with a placeholder or mismatched key, and refuses to overwrite a published release.
- 5H:
  - Ticket uniqueness: process-wide, `checked_add`, and exhaustion disables.
  - A stale or foreign ack never opens the gate, and a panic in release leaves the gate Disabled.
  - The mapper's `release_all` keeps physical tombstones (Rec::Swallow), so repeats after a handoff do not re-press or tap.
  - Local+device rules never reach peers (`rank()` returns None for `Device(_, Local)`, with a test).
  - Doctor, status and list honestly say the feature is inactive.
- 5E:
  - Decoding of tags 21/22/23 is bounded: the length is checked before reading, and UTF-8, trailing bytes, the 0/1 direction and the code range are all enforced.
  - `valid_text` rejects all Cc/Cf/Cs/Co/Cn, Zl/Zp, and Zs other than space (so no Enter, Tab, Esc, NEL, CSI, bidi or tag characters). Grapheme count is 1 and length is at most 8 bytes.
  - Old peers (main `link.rs:134`) skip unknown tags without closing. Malformed messages are only warn-logged.
  - TEXT is not in `caps::LOCAL`, and the daemon ignores Text/CharacterKey (`app.rs:503`), so nothing is injected.
  - The xkb utf8 buffer handles truncation and `checked_add` on the code. The Sender/Receiver held-key invariants hold under the 100k random test.
