# Encoded presentation admission precursor — stages A–D not reached

October 31, 2026 (Asia/Amman). Implementation commit `f93761a`.
This is **not the requested daemon → renderer IPC implementation**. It adds only
the [bounded payload codec and encoded Gate entry](../presentation-wire.md).
No production daemon/bridge/renderer calls the new API; production authorization
stays false and normal `--present=gnome` still refuses before surfaces/decoders.

## Proven

- Clean-environment `cargo test --offline -j 2 --workspace`: **1,179 passed,
  0 failed, 17 ignored**, 41 result records including doc tests (`workspace-test.log`).
  Ten new codec/admission tests supplement the existing eleven admission tests.
- Clean-environment `cargo clippy --offline -j 2 --workspace --all-targets --
  -D warnings`: passed (`workspace-clippy.log`). The same check with
  `--features one-mk-wall/gnome-bridge` passed (`bridge-clippy.log`).
- Component build with `--offline -j 2 -p one-mk -p one-mk-wall --features
  one-mk-wall/gnome-bridge --examples --bins` passed (`component-build.log`).
- `python3 docs/spikes/gnome/test_m1_gate.py`: six tests passed (`warning-gate.log`).
- The checked-in `qualify-component.py` passed on **GNOME Shell 50.1**, two
  virtual monitors, private system/session buses, **100 authenticated paused-fixture
  cycles**. Every exact child PID disappeared and its Shell child handle was reaped;
  private background URI unchanged; **zero new warning records** against the
  unchanged M1a-2 baseline. `result.json` and `component-run.log` record
  `production_playback_qualified: false`. `shell.log` is the positive run;
  `version-negative-shell.log` includes the separate deliberate version refusal.
  Authorization/retry reset through private Eval is laboratory scaffolding only.

The baseline and exact exercised implementation/bridge source hashes are in
`source-sha256.txt`; numerical results and attempted gates are in `validation.json`.
Archived text logs have trailing whitespace and blank EOF lines normalized for
the repository whitespace gate. Test/warning content is otherwise retained.

## Failed attempts and final environment

All attempts used the single worktree `target`, cleared environment, `-j 2`,
private HOME/runtime/state and no inherited live display or D-Bus variables.

1. `workspace-test-restricted.log`: sandbox socket creation failed with
   `Operation not permitted`; the shared port allocator mutex was then poisoned.
2. `workspace-test-long-ancestor.log`: approved socket access, but test fixture
   paths under the group-writable worktree ancestry triggered existing settings
   and hook ownership guards. This is not a product-code regression fix.
3. `workspace-test-long-socket.log`: private mode-0700 `/tmp` fixtures resolved
   those guards; one long-named mouse-mode fixture still failed under the long
   scratch path. Its pathname exceeded the Unix control socket path budget.
4. `workspace-test.log`: fresh short private `/tmp/wp.*` root; all workspace
   tests passed. No unrelated code was changed to make the tests pass.

The harness ran with approved private-socket/process access; it did not require
or access the owner's live GNOME session, X servers :0/:1, clipboard, real session
bus or `~/.config/one-mk`. Daemon fixtures use null input, discovery/notify off.
No formatter, external dependency fetch, cgroup provisioning, live-session install
or shipping authorization was performed.

## Not proven / not implemented

**A not reached.** Trusted real-daemon authority bootstrap, independently sourced
consent snapshots/subscriptions, actual daemon plan publication, renderer process
integration and teardown on revocation/daemon death remain absent. Supplying
identity/live consent arguments in a unit test is not transport authentication.

**B/C not attempted.** No imported VP9/AV1 WebM, GIF, APNG, animated WebP or still
was presented by the production renderer. No frame hashes/screenshots, 100-cycle
real-playback acceptance, visible wallpaper restoration, real daemon/renderer
SIGKILL, lock/unlock, hot-plug surfaces or suspend/resume are claimed.
**D remains false.** No production authorization was enabled. Paused fixture
cycles are only a regression check, not satisfaction of the requested gate.

## Cleanup

The harness finalizer removed its private session and copied extension sources.
`process-cleanup.json` records zero processes matching this run's exact private
HOME/runtime/temp/build markers. Four same-user service environments could not be
read; their recorded process start times predate the first test log, and none was
touched. `filesystem-cleanup.json` records deletion and verified absence of the
single worktree build target and both exact `/tmp` scratch roots after evidence
retention. No unrelated scratch or processes were removed.
