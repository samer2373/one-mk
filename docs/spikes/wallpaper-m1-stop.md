# 6D-M1: stopped at real-renderer presentation gate

Date: 2026-10-30. Author: Codex. Branch: `t3/wallpaper-import-scaffolding`.
Baseline: `7f331c9` (`Merge wallpaper design revision 2 (6E) into wave5`).
Owner authorized all milestones; this records **M1 incomplete**, not a release.
The frozen revision-2 contracts and receiver authority remain unchanged.

## Decision and evidence

Exercise the risky native integration first, before implementing an untrusted
media path or granting a peer any new authority. The real Rust `one-mk-wall`
binary decoded a generated VP9 fixture with GStreamer into the original 6B
private headless Mutter harness. It produced frames, but Mutter reported:

```text
(gnome-shell:2645131): libmutter-WARNING **: 20:32:43.556: Buggy client (one-mk-wall) committed initial non-empty content without acknowledging configuration, working around.
```

Design §11's lifecycle criterion requires no Shell warnings. This run therefore
does not pass presentation; the compositor workaround is not an acceptable
success. Apply §10's **stop/re-estimate** instruction. No importer, decoder
sandbox, consent protocol, production presenter, or platform host was built
after finding this failure. This does not establish that the architecture is
impossible, or diagnose whether the fault is in waylandsink, our integration,
the extension, or their interaction. A fresh investigation is required.

The [original machine-readable report](gnome/m1-presentation-original.json)
is preserved unchanged. Its `status: failed` originally resulted from a cleanup
exception; its stage strings describe narrow frame/extent observations, **not**
M1 acceptance. The protocol warning in its `shell_log_tail` independently fails
the gate. The harness now explicitly classifies that warning as a failure;
that updated classification was not rerun against Mutter after the stop.

Run began `2026-10-30T17:32:17.084752+00:00`, using host GStreamer 1.28.2,
Linux 7.0.0-38-generic, bubblewrap 0.11.1, two virtual 1920×1080 monitors.
The host runtime is **UNQUALIFIED**, not pinned/release-audited. Six selected
host plugins and their `ldd` closures are recorded; the string scan is not a
licence gate, SBOM, or proof that the complete runtime is acceptable.

| Observation | Result and limitation |
|---|---|
| Generated original fixture | VP9 WebM, 312,344 bytes; 3840×1080, 1,200 frames at 30 fps; BLAKE3 `b851b8199da653d839c7a5191bc2a9c280d1c9adf387ff833c21ae9ccbfa06b6` |
| First damage observed by supervisor | 164.978 ms; includes polling/IPC, **not** panel latency |
| Shell event delta | Spawn 795125520 → first frame 795125559 ms (39 ms); not physical presentation feedback |
| Initial / active | 3840×1080 buffer, two clones; damage count 2 → 62 over roughly two seconds; no focused window |
| Virtual unplug / replug | One then two clones; buffer still 3840×1080; damage counts 98 / 163 |
| Visible gap / scale / lock-restart | **Not measured**; retaining buffer extent is not gap-free rendering |
| Physical GPU / power / sync | **Untested**; no owner desktop or real clipboard accessed |

## Isolation and cleanup

All test processes use clean environments and throwaway directories in ignored
`target/wall-m1-lab`; GNOME uses the existing 6B harness's private session bus,
private system bus, HOME/XDG directories, memory settings and no Xwayland.
Only private extensions were enabled. The Rust risk probe checks the inherited
Wayland peer's UID, headless/no-X11 command and private runtime before loading
native libraries; ambiguous/live environments refuse with exit 3.

Linux namespace smoke outside the CLI sandbox passed both `unshare --user
--map-root-user --net --mount --pid --fork /usr/bin/true` and bubblewrap
`--unshare-all` with read-only `/usr`, private `/proc`, `/dev`, `/tmp` and an
empty environment. The earlier CLI-sandbox EPERM is **not** a host-isolation
failure. These are smoke tests only: no hostile-file, seccomp, Landlock,
filesystem/network escape, resource-limit or GPU-device sandbox qualification.

The original cleanup reported `[Errno 13] Permission denied` without a
traceback. Do not infer its exact source. All four recorded private process IDs
(2645089, 2645131, 2645098, 2649206), and PIDs extracted from the private log,
were subsequently checked absent. The private session/generated media were
removed manually; the exact owned stale `/tmp/dbus-g0mr0itTB5` socket was
verified as a socket belonging to the test UID and unlinked. Cleanup now handles
inaccessible child listings, repeatedly reaps adopted children, removes the
recorded private bus socket and records exception tracebacks. A synthetic owned
sleep child was terminated/reaped and its generated session files removed using
the revised cleanup. **The revised cleanup has not passed another GNOME run.**

## Exit criteria accounting

Only the stop/re-estimate criterion is met; the six delivery criteria remain
incomplete.

| Criterion | Status |
|---|---|
| Optional package, daemon/CLI, zero off-state work | **Partial:** separate default-disabled probe crate, no daemon linkage/startup, finite status/help/doctor and refusal paths. No main `one-mk wallpaper` surface, 6C integration, release package, or ten-minute idle/resource comparison |
| Safe import/poster | **Not implemented:** no untrusted imports accepted; no worker sandbox, all-format validation, caps/cancel/poster, malicious fixtures or fuzz campaign |
| Licence/SBOM/packaging | **Not implemented:** no pinned isolated distribution, replacement test, artifact/dependency negative test, SBOM or new cargo-deny/audit policy |
| Capability and receiver consent/revoke | **Not implemented:** daemon/wire capability and authority unchanged; no content/command peer path added; hostile peer consent tests still required |
| Geometry | **Not implemented:** no canvas/crops/property tests/goldens; reserved missing/asleep spaces remain a requirement |
| Native presentation | **Failed Linux risk gate:** real binary produced damage but protocol warning; scale/lock/kill-9/lifecycle/visible gaps unqualified. WorkerW/macOS hosts not implemented, not compile-qualified |
| Stop and re-estimate | **Done:** this report and updated feature-plan row; M1 remains blocked on presentation investigation, not complete |

## Reproduction and troubleshooting

Do not run the probe against a live GNOME session. Normal/default builds accept
only finite `status`, `doctor`, and `--help`; doctor and unavailable operations
exit 3. There is no production import/playback fallback. Do not enable host
codec discovery, install a real-user extension, or substitute FFmpeg/libav.

For a separately authorized follow-up investigation, create owned mode-0700
`target/wall-m1-lab/{home,run,one-mk}` directories and use this environment for
every command (including builds; use the actual checkout's absolute path):

```bash
env -i PATH="$PATH" HOME="$PWD/target/wall-m1-lab/home" \
  XDG_RUNTIME_DIR="$PWD/target/wall-m1-lab/run" \
  ONE_MK_HOME="$PWD/target/wall-m1-lab/one-mk" \
  CARGO_HOME=/home/samer/.cargo RUSTUP_HOME=/home/samer/.rustup \
  TMPDIR="$PWD/target/wall-m1-lab" \
  cargo build --offline -j 2 -p one-mk-wall --features gnome-risk-probe
```

Then invoke `python3 docs/spikes/gnome/m1-risk-probe.py` through the same clean
environment. It refuses an existing session rather than reusing/killing it.
Namespace operations may need permission outside a restricted CLI sandbox.
Logs remain in ignored lab storage; generated session media/plugins are removed.
This is a future diagnostic recipe, **not** a recommendation to resume the
stopped implementation or a safe general-purpose media runner.

## Hostile self-review and next estimate

The dynamic FFI loader and trusted-fixture hash are laboratory conveniences,
not a security boundary: same-UID mutation/TOCTOU, transitive native library
loading, resource exhaustion and host-library replacement are not addressed.
No untrusted media may enter this code. Default feature-off is not a measured
ten-minute daemon/compositor idle proof. Successful cross compilation is not
native host behavior. Synthetic cleanup is not a 100-cycle lifecycle test.
No independent reviewer, legal audit, fuzz coverage, or hardware acceptance
has been substituted by this self-review.

Planning judgment, not measured delivery throughput or calendar dates:

| Remaining work | Engineer-weeks |
|---|---:|
| M1 including native presenter investigation/qualification | 14–22 |
| M2 GNOME multi-computer playback | 10–16 |
| M3 Windows/macOS and power | 16–24 |
| M4 HTTPS/gallery/UI | 10–16 |
| M5 release qualification | 14–22 |
| Integration/security reserve | 8–14 |
| **Total remaining** | **72–114** |

The extra 2–4 M1 weeks are an uncertainty allowance, not a promise that the
protocol issue is small. Re-estimate again after root-cause reproduction and
warning-free lifecycle runs, then after real sandbox/codec replacement gates.
Real-hardware owner acceptance remains design §11: GNOME HiDPI/rotation/lock/
sleep/DING/hot-plug/Overview layering and gaps; Windows WorkerW/Explorer and
macOS Spaces/desktop levels; iGPU/hardware decode, VRAM/wakeups/power and input
latency; filmed multi-machine wired/Wi-Fi P95/P99, pause/off and 24-hour soak.
**Do not ask the owner to exercise this failed risk binary on their desktop.**

## Validation record

All commands used clean environments, throwaway HOME/XDG/ONE_MK_HOME, offline
Cargo with `-j 2`, and disk-backed worktree scratch. No formatter was run.

| Check | Actual result |
|---|---|
| Failing-first unsafe-root guard | Initial permissive guard failed the rejection test; implemented guard passed. The original failing transcript was not retained |
| `cargo test --offline -j 2 -p one-mk-wall --features gnome-risk-probe` | **Pass:** one guard and three real-binary integration tests; refusal/status/help/doctor; every invocation gets a fresh private environment, is reaped, and leaves no files |
| `cargo test --offline -j 2 -p one-mk-wall -p one-mk-wall-sync` | **Pass:** three default-binary tests and twelve existing sync integration tests; no production sync integration claimed |
| `cargo clippy --offline -j 2 --workspace --all-targets -- -D warnings` | **Pass** |
| Feature-enabled probe clippy, all targets, `-D warnings` | **Pass** |
| Windows/macOS cross clippy, all targets, `-D warnings` | **Pass** for `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`, each with `one-mk-input`, `one-mk-proto`, `one-mk-ctl`, `one-mk-app`, `one-mk-tray`, `one-mk-wall`, `one-mk-wall-sync`. Native host implementations remain absent; runtime **UNTESTED** |
| `cargo test --offline -j 2 --workspace` inside CLI sandbox | **Fail:** daemon 231 passed / 129 failed / 2 ignored; first failure loopback bind `EPERM`, then poisoned locks and other failures |
| Same full workspace command outside CLI sandbox | **Fail:** daemon 302 passed / 58 failed / 2 ignored; failures include control/capture expectations, connection timeouts, hook/settings privacy checks. One concrete diagnostic: `target is writable by its group`. No unrelated application code was changed to suppress these failures |
| Private disk-backed short `/tmp` retry | **Unable to execute tests:** bubblewrap mount/PID namespace starts, but Cargo's rustc subprocess returns `Permission denied`; not a passing workspace run |
| Default binary smoke measurement | `status` exit 0, `/usr/bin/time -v`: 0.00 s user/system/wall at printed resolution, peak RSS 2,240 KiB, one voluntary context switch, zero filesystem output. This is a **finite command**, not a ten-minute idle measurement or claim of literally zero execution CPU |
| Daemon dependency tree | `cargo tree --offline -p one-mk` contains no `one-mk-wall` package; default binary `ldd` lists only libc/libgcc/loader (no GStreamer) |
| Cleanup smoke | Revised cleanup terminated/reaped one owned synthetic sleep child and removed its generated session files; original recorded GNOME PIDs absent, exact stale private bus socket removed manually |

Small diagnostic logs remain under ignored `target/wall-m1-lab`. The original
presentation report above is the committed primary evidence. **Full workspace
tests are not green**, so the quality bar is not satisfied independently of
the native presentation stop. No kill-9 end-to-end media path, sandbox escape
suite, ten-minute off-state trace or physical acceptance was completed.
