# Hostile security review: one-mk optional live wallpaper (wave5)

Reviewer: Claude (independent, read-only). Branch `wave5`, reviewed at 1d66924 (HEAD moved to 88edae1 during the review; none of the in-scope files changed in between).
Scope: (A) `crates/wall-import`, licence/SBOM gates, cargo-deny. (B) consent and canvas: `crates/one-mk/src/wallpaper.rs`, `crates/proto/src/wallpaper.rs`, `crates/wall-canvas`, settings. (C) GNOME presenter spike: `crates/wall-presenter`, `docs/spikes/gnome/*`.

Experiments I ran:
- A bwrap probe with the importer's exact namespace, `/dev` and `/tmp` flags, plus a trivial `/usr` bind.
- About 302k randomized and prime-extreme canvas cases, run in an overflow-checked debug build in `/tmp/rvw-canvas`.
- `cargo tree` feature-unification checks.

I did not run the GNOME harness, a renderer or the importer against the owner's session.

---

## Ranked findings

### 1. MEDIUM-HIGH: the sandbox's `/dev` is a writable tmpfs sized at half of RAM (about 15 GiB here). It bypasses the "16 MiB scratch" and every memory limit.
- **Where:** `crates/wall-import/src/sandbox.rs:155` (`"--dev", "/dev", "--size", "16777216", "--tmpfs", "/tmp"`). `--size` applies only to the following `--tmpfs /tmp`.
- **Evidence (bubblewrap 0.11.1, same namespace flags):** `/proc/self/mounts` shows `tmpfs /dev tmpfs rw,...mode=755,uid=1000` with no size option. `df` shows 15,979,636 KiB available on `/dev` and `/dev/shm`, against 16,384 KiB on `/tmp`. Writing 1 MiB to `/dev/shm/probe` and to `/dev/probe2` both succeeded.
- **Scenario:** a hostile VP9/AV1 file exploits libvpx, libaom or matroskademux. This decoder code is the reason the sandbox exists. The code then writes 512 MiB files (`RLIMIT_FSIZE` is per file and `RLIMIT_NOFILE` only limits open descriptors) into `/dev/shm` until about 50% of RAM is tmpfs pages.
  - Those pages are not counted by `RLIMIT_AS`. They are charged to the owner's session cgroup and stay pinned until the 600 s wall timeout tears the namespace down.
  - On the owner's dev box, which already OOMs, this takes down the desktop session.
- **Doc conflict:** WALLPAPER-IMPORT.md claims "private proc/dev and a 16 MiB scratch tmpfs". The aggregate-memory caveat it does make covers decoder threads and RLIMIT_AS, not a writable 15 GiB tmpfs.
- **Fix:**
  - Mount `/dev` read-only after setup: `--dev /dev --remount-ro /dev`. If GStreamer needs `/dev/shm`, give it its own `--size 1048576 --tmpfs /dev/shm` before the remount. Alternatively, build a minimal `/dev` with `--dev-bind /dev/null /dev/null` and similar binds.
  - Add the delegated cgroup memory/pids limit the doc already lists.
  - Extend `check_worker` so doctor proves `/dev` is not writable and that the total tmpfs capacity is bounded.
- **Tests:** none would fail today. `check_worker` probes only `/sandbox-write-test`, `/etc/passwd` and `/home`.

### 2. MEDIUM: an unreadable `[wallpaper]` section makes the whole `settings.toml` unreadable. The daemon then starts with all-default settings, which re-enables clipboard sharing the user turned off.
- **Where:**
  - `crates/one-mk/src/wallpaper.rs:35` (`deny_unknown_fields`) and `:43` (requires `version == 1`, at most 8 grants, lowercase 64-hex keys, revision not 0).
  - `crates/one-mk/src/app.rs:734` (`Settings::load_or_default`) and `settings.rs:509-514` (falls back to `Settings::default()` on any parse error).
  - The top-level `Settings` has no `deny_unknown_fields`, so `[wallpaper]` is the only strict section.
- **Scenario:**
  - The user downgrades from a future build that writes `version = 2` or a new wallpaper key. Or a hand edit adds a 9th grant or an uppercase fingerprint.
  - `Settings::load` fails for the whole file, and the daemon logs a warning and runs on defaults: `clipboard.text/html/image = true`, `allow_pull = false`. Edge, gaming, hooks, power, lock-sync and tuning choices are all silently dropped. A user who disabled clipboard sharing has it re-enabled.
  - `Settings::update` also fails, so neither `one-mk wallpaper disable` nor `consent revoke` can repair the file. A running daemon's `reload_settings` fails, which keeps stale grants alive in memory.
  - Related: `consent grant` stores `device.fingerprint.clone()` verbatim (`wallpaper.rs:240`). `TrustStore` does not canonicalise fingerprints, and `Settings::save` serialises without re-validating. A hand-edited uppercase or garbled fingerprint in `devices.toml` therefore makes the CLI write a `settings.toml` that it then refuses to read.
- **Fix:**
  - Make wallpaper policy fail closed locally: on a section error, set `WallpaperSettings::default()` (off, no grants) and keep the rest of `Settings`. For example, deserialize `wallpaper` as a raw `toml::Value` and convert it separately.
  - Canonicalise grants through `Fingerprint::from_str(..).to_string()` before saving.
- **Tests:** `old_settings_migrate_to_off_with_no_consent` asserts that `[wallpaper] version=2` errors. Nothing tests the blast radius on the other settings.

### 3. MEDIUM: the runtime's own loader and libc run the seccomp installer, and runtime ownership and ancestors are not checked. Runtime integrity therefore equals sandbox integrity.
- **Where:**
  - `sandbox.rs:28-74` (`validate_runtime` checks only `mode & 0o022` on the root and its members; no uid check, no ancestor check).
  - `sandbox.rs:152-159` (binds the runtime as `/` by path after validation; `LD_LIBRARY_PATH=/lib`).
  - The dynamically linked `/worker` (the host binary) is then executed by the runtime's `/lib64/ld-linux*.so` and libc. The lab script copies them in via `ldd`.
- **Scenario:**
  - Runtime under another user (any user-chosen path whose owner is another local account with mode 0755): that owner swaps `lib/libc.so.6` between `validate_runtime` and bwrap's bind. This is a TOCTOU; the hashes were checked earlier by path.
  - Runtime under a directory others can write (e.g. `/srv/shared/rt`): the parent directory is renamed and replaced.
  - In both cases the injected libc or loader runs before `main`, so `worker_restrictions()` (no_new_privs and seccomp) never runs. The decoder then runs with namespaces only, and nested user namespaces are allowed: `max_user_namespaces` is 2147483647 inside, and `unshare -U` worked in my probe.
  - The doc acknowledges a malicious packager and same-user mutation. It does not cover cross-user ownership or ancestor swaps.
- **Fix:**
  - Install the filter from outside the runtime with bwrap's `--seccomp FD` / `--add-seccomp-fd`, so the BPF program is loaded before `execve` and independently of the runtime's loader.
  - Require uid 0 or the caller's euid on the root, every member and every ancestor.
  - Open the root once and bind `/proc/self/fd/N`, so bwrap mounts exactly the directory that was validated.

### 4. MEDIUM: the 1e9 decoded-pixel budget makes most of the advertised admission tier unusable. The budget is enforced only after the decode work is done, and no test exercises it.
- **Where:** `lib.rs:14`; `decode.rs:51` (images); `decode.rs:97` (WebM).
- **Effect:**
  - 4096×2160 sources are capped at about 113 frames (3.8 s at 30 fps).
  - 1920×1080 sources are capped at about 482 frames (16 s at 30 fps).
  - A normal 1080p 60 s loop is rejected with "decoded pixel/frame budget exceeded" (no "convert first" hint), after up to 1e9 pixels of decode work.
  - This contradicts the documented 36,000 frames / 600 s tier.
- **Test gap:** the only "bomb" fixture (`binary.rs:247`) sets the GIF dimensions to 0xffff and is rejected by admission. Removing the `pixels > MAX_DECODED_PIXELS` check, the WebM `frames*W*H` check or the `image::Limits` would not fail any test.
- **Fix:**
  - Reject at admission when `frames × W × H` exceeds the budget, with a "convert first / lower resolution" message.
  - Decide whether the budget should be a per-second rate rather than a total.
  - Add fixtures with valid dimensions that exceed the budget for the GIF and WebM paths.

### 5. LOW-MEDIUM: the seccomp filter is a denylist and does not match the doc's claims.
- **Where:** `sandbox.rs:79-98` (`worker_restrictions`) and `:123-140` (`deny_subprocesses`).
- **Gaps:**
  - `add_key` and `request_key` are reachable; only `keyctl` is denied, yet the doc says "keyctl ... denied". This kernel keyring attack surface is reachable from the gst-launch process that parses untrusted media.
  - The new mount API is not denied: `fsopen`, `fsconfig`, `fsmount`, `fspick`, `move_mount`, `open_tree`, `mount_setattr`. The same goes for `pivot_root` and `chroot`, although the doc says "namespace/mount changes ... denied".
  - `clone`/`clone3` with `CLONE_NEW*` is allowed by the base filter. `clone3` is only blocked once `deny_subprocesses` is installed. The only code exposed in that window is the safe-Rust `inspect()`, so this is defense in depth.
  - `accept4` is missing.
  - `execve`/`execveat` stay allowed on the image path the doc describes as "cannot create subprocesses".
  - No `--disable-userns`.
- **Mitigation:** `--cap-drop ALL` plus `deny_subprocesses` block most of these for the C decoder, which is why this is not higher.
- **Fix:** a default-deny allowlist (glibc and GStreamer need a small set), passed via bwrap `--seccomp` (see finding 3). Deny `clone` namespace flags in the base filter too. Add `--disable-userns` where bwrap ≥0.8 is available.

### 6. LOW-MEDIUM: the parent trusts the worker's poster and metadata beyond range checks.
- **Where:** `transaction.rs:97-110`.
- **What is checked:** only that the poster length is at most 172,832 bytes and starts with `P6\n`, and that the frame and duration ranges are plausible.
- **Scenario:** a compromised worker writes `poster.ppm` with a header of `P6\n65535 65535\n255\n` and a tiny payload. The future UI or daemon that loads posters then allocates 12 GiB or misparses. Likewise `record.json` can claim any format, or a 36,000-frame / 1 ms timeline (the 60 fps rule is not re-checked), and it is persisted as a "validated" object.
- **Fix:**
  - Parse the PPM header strictly: exactly `P6\n{w} {h}\n255\n`, 1≤w≤320, 1≤h≤180, length equal to header + w·h·3.
  - Re-check the fps rule.
  - Optionally re-run the pure, decoder-free `inspect()` in the parent to cross-check format, dimensions and frame count.

### 7. LOW: `one-mk wallpaper import` panics when it is not the first argument.
- **Where:** `main.rs:450-453` pre-dispatches only when `raw[0]=="wallpaper" && raw[1]=="import"`. `wallpaper.rs:218` has `WallpaperCommand::Import { .. } => unreachable!(..)`.
- **Repro (by code path):** `one-mk -v wallpaper import f.gif ...`, `one-mk --home D wallpaper import ...`, or `one-mk wallpaper --json import ...` (`--json` is global). Each goes through clap and `Settings::load`, then hits `unreachable!` and exits with code 101. This also breaks the documented promise that import "dispatches before configuration".
- **Fix:** have `execute()` call `wallpaper_import::run(arguments)`, or make the pre-dispatch skip the global flags.

### 8. LOW: building the whole workspace pulls GIF and WebP decoders into the daemon, and the "main daemon contains no media decoder" message is inaccurate.
- **Feature unification:** `cargo tree --workspace -e features -i image` shows image features `gif`, `png` and `webp`. README:1148 documents a plain `cargo build --release`, which unifies the importer's `gif`/`webp` features into the daemon's `image`.
- **Why it matters:** arboard's Wayland `get_image` uses `with_guessed_format()`. A local app offering `image/png` with GIF or WebP bytes is then decoded inside the daemon, with no `Limits` set.
- **Release builds are unaffected:** `release.yml:51` builds with `-p`.
- **The message:** the daemon always contains `image`/`png` for the clipboard, so `wallpaper_import.rs:11`'s message is not literally true.
- **Fix:** set `default-members` to exclude `wall-import`, or move it to its own workspace. Reword the message to "no wallpaper/video decoder".

### 9. LOW: any wallpaper decode error closes the whole KVM control link.
- **Where:** `link.rs:136`.
- **Problem:** a future envelope version or kind under the same `caps::WALLPAPER` bit (256) disconnects keyboard and mouse sharing with older peers in a reconnect loop. The behaviour is documented as intentional.
- **Fix:** close only on length or framing errors and skip unknown version or kind values. Alternatively, commit to a new capability bit for every envelope revision.

### 10. LOW: a peer's `ConsentRevoke` is undone by a daemon restart or by disable then enable.
- **Where:** `wallpaper.rs:133`, `:76-86`.
- **Problem:** the revoked state lives in memory only. WALLPAPER-CANVAS.md says "only a local regrant revives that session". Impact is minimal, because only an already-consented coordinator regains its session. Fix the doc or persist the stop.

### 11. LOW (GNOME spike, C): bridge name squatting combined with untyped GVariant parsing in the Rust renderer.
- **Where:**
  - `extension.js:119-120` owns `org.onemk.WallpaperBridgeSpike1` with `DO_NOT_QUEUE`. On failure it only logs 'lost bus name' and keeps running.
  - `bridge.rs:178-190` calls `Hello` and subscribes to signals by that well-known name.
  - `bridge.rs:20-35` and `:55` walk the replies and signals with `g_variant_get_child_value`, `n_children` and `get_int32`, without `g_variant_is_of_type`.
- **Scenario:** any unsandboxed process in the session claims the name before the extension is enabled. Security probe 3 tested only the reverse order. The renderer then takes Layout and PauseHint from the squatter:
  - A non-container value gives a NULL child, and `g_variant_n_children(NULL)` crashes.
  - A 16 Mpx Layout forces repeated allocations of up to 256 MB (`surface.rs:77`, `:54-65`).
- **Status:** stop gate 3 (unique-name binding) already acknowledges this in general.
- **Fix:** type-check every variant (`(ua(iiii))`, `(a(iiii))`, `(bs)`). Pass the extension's unique name to the renderer through its spawn argv/env. The extension should refuse to spawn when it does not own the name.

### 12. LOW (C): `FrameReady` image mode blocks or OOMs gnome-shell.
- **Where:** `extension.js:521-524`.
- **Problem:** `GLib.file_get_contents($XDG_RUNTIME_DIR/one-mk-spike-frame.rgba)` follows symlinks, reads the whole file on the compositor's main thread, and checks the size only afterwards. The renderer is the media-facing process, and it may call this method. It can point the file at a FIFO, which freezes the Shell forever, or at `/dev/zero` or a huge file, which exhausts the Shell's memory.
- **Status:** only reachable in Eval-selected 'image' mode.
- **Fix:** open with `O_NOFOLLOW`, `fstat` for a regular file of exactly `IMG_BYTES`, then read asynchronously. Better, use a memfd passed over the bridge.

### 13. LOW (C): a replaced renderer window keeps DESKTOP type without hide enforcement.
- **Where:** `extension.js:258-259`, `_dropRenderer` at `:337-347`.
- **Problem:** when the client opens a second toplevel, the previous window is detached but not closed. It stays `Meta.WindowType.DESKTOP`, hidden from the window list, spanning the whole canvas, and its `notify::visible` re-hide handler is disconnected. Mutter can re-show it under all windows.
- **Scenario:** a misbehaving or compromised renderer with a non-empty input region intercepts desktop clicks, including those meant for DING icons.
- **Fix:** `old.win.kill()` or `delete()` on replacement, or refuse more than one live window per client.

### 14. LOW (test and gate quality)
- **Hard-coded status fields:** `status()` returns the constants `"timers": 0, "children": 0` (`wallpaper.rs:144`). The e2e assertions on them (`wallpaper_e2e.rs:190, 216-217`) can never fail; only the CPU-tick check is real.
- **Weak flood assertion:** the e2e flood check `rejected >= 10` (`:167`) passes even if the spoof, wrong-epoch or rate-limit checks are removed. Unit tests do cover those.
- **Sandbox tests silently skipped:** all sandbox, kill and descendant tests return early when `ONE_MK_WALL_TEST_RUNTIME` is unset, so CI never exercises bwrap, seccomp or kill cleanup (documented). No test kills the import parent with SIGKILL, so the orphan behaviour after parent death is unproven.
- **Weak cargo negative test:** the licence-gate test (`check-wallpaper-artifact.py:139-144`) passes on any `ValueError`, including the "exact package ... required" error. Assert on the message.
- **Import-time gate weaker than the script:** `validate_runtime` (the gate that actually runs at import) lacks the Python gate's exact plugin allowlist and ELF NEEDED/RPATH checks. An extra permissively labelled plugin, for example `plugins/libgstopenh264.so` (BSD-2, an H.264 decoder), passes, and its `plugin_init` runs in the sandbox during the registry scan.
- **Narrow cargo-deny ban list:** cargo-deny bans only `ffmpeg-next`, `ffmpeg-sys-next`, `libmpv`, `x264` and `x265`. The Python regex catches other names containing "ffmpeg", but cargo-deny does not.
- **Duplicate import exits 5:** a re-import exits 5 (RENAME_NOREPLACE) instead of an idempotent success that reports the existing object.

---

## Verified OK
- **Admission parsers (GIF/APNG/WebP/WebM):**
  - All offsets are bounds-checked through `Cursor::take` with `checked_add`; field reads slice fixed lengths. I found no panic paths.
  - The EBML vint length is capped, and unknown size is accepted only for Segment. Lacing, BlockGroup, encryption, extra tracks and attachments are rejected.
  - Polyglot trailers are rejected: the GIF trailer must be at EOF, PNG `IEND` must be at EOF, the WebP RIFF length must match the file, and WebM rejects trailing elements.
  - Metadata is capped at 256 KiB. Frame, duration and fps maths uses u64/i64 with checked operations. The float duration is checked for finiteness.
- **TOCTOU:**
  - Images are inspected and decoded from the same in-memory bytes.
  - WebM is decoded by gst from a read-only bind of an immutable snapshot inside a 0700 staging directory. The parent re-hashes the snapshot after the worker finishes.
- **Paths:**
  - Sources are opened by an `openat(O_NOFOLLOW|O_CLOEXEC|O_NONBLOCK)` walk; `..` components are refused; FIFOs and devices are refused by `is_file`.
  - The output directory is opened by descriptor, must be owned with mode 0700, and is locked with `flock`. Staging is created through `/proc/self/fd`.
  - Commit uses `renameat2(RENAME_NOREPLACE)`, so symlinks are never followed or replaced. Stale cleanup uses std's symlink-safe `remove_dir_all`.
- **Process isolation:**
  - No descriptor leaks into the sandbox (everything is CLOEXEC) and the environment is cleared.
  - Namespaces: user, PID, net, IPC and UTS. Also `--new-session`, `--die-with-parent` and `--cap-drop ALL`.
- **Seccomp:**
  - The architecture check kills x32 and i386 entry. `socket` returns EPERM, which doctor verifies.
  - I verified the `deny_subprocesses` jump offsets by hand.
  - It fails closed when bwrap or seccomp is missing; there is no fallback.
- **Worker I/O and cleanup:**
  - Worker stdout and stderr are bounded (1 MiB and 16 KiB).
  - The worker is killed on timeout, cancel or overflow, and both reader threads are joined.
- **Unsafe code:** the signal handler and the `pre_exec` `setrlimit` closure are async-signal-safe.
- **Daemon:** has no dependency on `wall-import` or GStreamer (in release `-p` builds; see finding 8). It runs the sibling helper with `env_clear` via `exec`.
- **Canvas:**
  - About 302k randomized and prime-extreme cases (coordinates ±1e6, sources up to 1e6, SAR up to 1e4, all scales, rotations and fits) under overflow checks: no panic, no i128 overflow, no sample out of range.
  - Layout/resolve arithmetic is checked. Cycles are detected (Ambiguous or Inconsistent).
- **Wire envelope:** exact 112-byte length, zero reserved bytes, kind and canvas bounds checked; epoch and sequence must be non-zero.
- **Receiver:**
  - Requires the feature enabled, explicit receiver-side consent, `coordinator == TLS fingerprint` and the current link id.
  - The epoch comes from a CSPRNG and rotates on (re)connect, regrant and restart. Sequences must strictly increase. Self-grant is refused.
  - Rate limits: 64/s per link and 16/s per peer. There are no timers, and off-state holds no receivers and sends no announcements. Forgetting a device removes its grant.
- **Extension:**
  - Window recognition uses `Meta.WaylandClient.owns_window`, not title or app-id; probe 5 confirms spoofing fails.
  - Timers and signals are removed on destroy. The launcher is closed in `finally`.
  - Restart budget: 1/2/4 s backoff, 3 attempts per rolling 60 s, latched exhausted state shared across re-enable. The gate checks `cap_after_reenable` (`m1a-matrix.py:254`).
- **Presenter probe:** only runs with feature `gnome-risk-probe`, under strict private-session environment checks plus `SO_PEERCRED` verification of the inherited Wayland socket. The probe binary refuses by default.
- **Licence script:** the SPDX parser fails closed on missing or malformed expressions. The runtime negative self-test asserts the specific error and can fail.
