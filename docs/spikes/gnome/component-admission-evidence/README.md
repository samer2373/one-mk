# Admission precursor and component lifecycle evidence

October 31, 2026 (Asia/Amman), workspace 0.1.15 plus importer cgroup slice.
Admission implementation: `6299fd1`. Exact exercised source and unchanged warning
baseline hashes are in `source-sha256.txt`.

Archived text logs have only trailing whitespace/blank EOF lines normalized for
the repository whitespace gate. Warning text, stacks and test results are
otherwise retained; the warning comparison ran on the original captured logs.

## Passed

- Clean-environment `cargo test --offline -j 2 --workspace`: 1,169 passed,
  0 failed, 17 ignored, 40 result records including doc tests. Eleven new tests
  cover presentation admission and lifecycle intent. The two native delegated
  cgroup tests remain ignored; runtime-conditional tests are not decoder evidence.
- Clean-environment `cargo clippy --offline -j 2 --workspace --all-targets --
  -D warnings`: passed (`workspace-default-clippy.log`). Also passed with
  `--features one-mk-wall/gnome-bridge` (`workspace-clippy.log`), including both
  test-only bridge examples. No formatter was run.
- `python3 docs/spikes/gnome/test_m1_gate.py`: six passed, including new-warning
  identity and changed-stack detection (`warning-gate.log`).
- `python3 docs/spikes/gnome/qualify-component.py`: passed against **GNOME Shell
  50.1**, two virtual monitors, **100 authenticated paused-fixture child cycles**,
  each child handle reaped and exact PID gone, no private background URI change,
  **zero new warning records** against the unchanged M1a-2 baseline. `result.json`
  records `production_playback_qualified: false`. `shell.log` is the positive
  run; `version-negative-shell.log` additionally includes the intentional
  exact-version refusal after changing only the private extension's metadata.
- Existing CLI install/status/uninstall, unauthorized caller, unicast isolation,
  bridge signatures/layout/state/rates, name squatting and retry exhaustion
  assertions also passed. Private Eval authorization/reset of the retry budget
  is test scaffolding, never a shipped permission path.

The first restricted harness start failed and finalized cleanup; its traceback
is retained. The identical isolated command then passed with approved private
socket/process access. No live GNOME session, X server, clipboard, real D-Bus
session or owner one-mk config was used. Cargo used only this worktree's `target`,
with cleared environment and private HOME/runtime/one-mk state. The harness used
its own disposable environment and private buses. Source was built with
`cargo build --offline -j 2 -p one-mk -p one-mk-wall --features
one-mk-wall/gnome-bridge --examples --bins`.

## Not proven / not implemented

This is a **partial precursor**, not any of the requested end-to-end playback
gates. The new I/O-free `Gate` is not called by the daemon or renderer and does
not authenticate bus credentials itself. No daemon presentation-plan transport,
wire extension, media/cache resolver or playback pipeline was added. No normal
production authorization becomes true. `--present=gnome` still refuses.

The child fixture reports Paused and never opens a surface or decoder. These
100 cycles do not prove visible playback, poster/background restoration,
daemon stop/crash/SIGKILL behavior, lock/unlock, suspend/resume or hot-plug
surface re-slicing. Gate tests simulate those intents only. VP9/AV1 WebM,
GIF/APNG/animated WebP and still-image production playback remain unproven, as
do physical GPU/HiDPI and other native platforms.

A read-only inspection outside the sandbox found a writable-mounted cgroup v2
filesystem but root-owned root controls not writable by the invoking UID. The
sandbox exposes that mount read-only. No explicit private delegation was supplied
or provisioned, and no controllers or memberships were changed. Do not interpret
the earlier run's read-only sandbox observation as proof of a read-only host.

The harness finalizer removed its session/copied sources. All command sessions
exited and the host-side environment-marker scan found no remaining lab processes
(`process-cleanup.json`). Eighteen exact test-owned `/tmp` directories were
removed after their three test PIDs and environment-bound children were verified
gone (`test-scratch-cleanup.json`). No manual `/tmp` laboratory was created.
The single worktree `target`, including every private HOME/runtime/state/build
directory, was deleted after retaining this evidence; absence was checked.
