# Wave 6 M1 implementation slices — NOT M1 completion

## Latest slice: presentation admission precursor, not playback

October 31, 2026 (Asia/Amman), workspace 0.1.15 plus the cgroup importer slice.
Implementation commit `6299fd1` adds the I/O-free admission core. The requested
end-to-end daemon/media/lifecycle slice is **not completed** by this delivery.

**Done:** bounded plans carry an imported-media content hash, consent revision,
authority generation, monotonic sequence, local-layout serial, frozen canvas,
ordered monitor slices and loop/manual-pause state. The gate compares pinned
caller identity (unique name/UID/PID), waits for the initial pause snapshot,
rejects stale/invalid plans by clearing admitted intent, composes pause reasons,
requires fresh plans after layout/suspend changes, and makes owner loss or consent
revocation terminal for that session. Missing/asleep output space never shrinks
the frozen canvas. Eleven failure-path tests cover this model.

**Not implemented:** this core is not called by the daemon, D-Bus bridge or
renderer. A future transport must obtain credentials independently, reconcile
live consent and actually tear down surfaces/decoders when intent is absent.
The existing Shell bridge XML is unchanged (no paths/media channel). There is
no new daemon-plan IPC, media resolver, production decoder/surface pipeline,
or normal-build authorization path. Production authorization stays false and
`--present=gnome` refuses. No copied Muro/Aura/Hanabi code was introduced.

**Proven in the isolated laboratory:** GNOME Shell 50.1 with two virtual
monitors passed the existing bridge/CLI conformance and negative cases plus
100 authenticated **paused-fixture** child start/stop/GC cycles. Each exact child
PID disappeared and its handle was reaped; the private background URI was
unchanged. Zero new warning records against the unchanged M1a-2 baseline.
The restricted start failed; the approved private-socket rerun passed, rather
than substituting unit tests for a harness run. Private Eval authorizes the
test fixture and resets its retry budget; a separate negative still verifies
the unmodified real restart cap. No owner desktop/session bus was accessed.

**Not proven:** those children never create surfaces or decode media. Thus
there is no production VP9/AV1 WebM, GIF/APNG/animated WebP or still playback
evidence, no 100-cycle visible-playback/restoration gate, and no actual daemon
stop/crash/kill, lock/unlock, suspend/resume or hot-plug surface acceptance.
State-machine tests are not evidence for OS lifecycle hooks. Physical GPU/HiDPI
and Windows/macOS remain unqualified. No explicit private cgroup delegation was
provided/provisioned; the native cgroup tests remain ignored. Outside the sandbox,
cgroup v2 is mounted read/write but its root controls are not writable by the
invoking UID; the earlier read-only observation was the sandbox view, not proof
of a read-only host. No controller or membership changes were made.

**Validation after source changes:** clean-env `cargo test --offline -j 2
--workspace`: **1,169 passed, 0 failed, 17 ignored**, 40 result records including
doc tests. Clean-env `cargo clippy --offline -j 2 --workspace --all-targets --
-D warnings`: passed, also with `--features one-mk-wall/gnome-bridge`. The warning
gate's six Python tests passed. `git diff --check` passed. No formatter ran.
Conditional audited-runtime tests returning `ok` do not qualify native decoding.
Builds used one worktree `target`, cleared HOME/runtime/one-mk environment and
`-j 2`; daemon test fixtures use the null backend with discovery/notify disabled.

Exact logs, source/baseline hashes and reproduction/interpretation notes are in
[component-admission-evidence](../spikes/gnome/component-admission-evidence/README.md).
M1 remains incomplete and the 16–26 engineer-week low-confidence estimate is
unchanged. The historical resource-only report below is retained, not relabelled
as evidence for this new run.

**Final cleanup for the latest slice:** both cargo command sessions and the
private harness exited successfully. The harness removed its session and copied
extension sources. A host-side process-environment check found zero remaining
processes bearing this run's worktree lab markers (`process-cleanup.json`).
Eighteen exact `/tmp` test directories left by three completed test executables
were removed after verifying ownership, absent test PIDs and no environment-bound
children (`test-scratch-cleanup.json`). No manual `/tmp` lab was created. After
retaining evidence, the single worktree `target` (including all private HOME,
runtime, one-mk state and build products) was deleted and its absence checked.
No owner configuration, clipboard, live display or real session bus was touched.

## Previous slice: importer aggregate-resource containment

Date: October 31, 2026 (Asia/Amman). Workspace version 0.1.15, branch
`t3/wallpaper-m1-playback-lifecycle`. This re-run changes implementation; it does
not stop at inspection. The delivered slice is the importer aggregate-resource
gate. It is not an enterprise-ready playback delivery or milestone acceptance.

## Done

- Linux importer/doctor accept `--cgroup-root`, including through the existing
  environment-cleared `one-mk wallpaper` helper dispatch. Public `ImportOptions`
  carries the optional path; absence is a fail-closed unavailability, not an
  rlimit-only fallback. Windows/macOS remain unavailable.
- The root is pinned by an O_NOFOLLOW component walk, ownership/mode checks and
  cgroup v2 filesystem validation. It must already have memory/pids subtree
  controllers enabled and be a domain. No controller enablement, parent migration,
  D-Bus call or service-manager invocation is introduced.
- A fresh child group gets read-back-checked 1 GiB charged memory, zero swap,
  whole-group OOM and 64-task limits. The launcher attaches using a pinned
  membership FD before exec; membership failure prevents exec. Descendants and
  threads share the limits. A root-directory flock prevents simultaneous workers
  using the same delegation, including different output directories.
- The external seccomp filter rejects explicit huge-page mappings; doctor checks
  EPERM. A filter-interpreter regression verifies ordinary mmap stays allowed
  while MAP_HUGETLB is denied, without allocating huge pages on the host.
- Teardown kills the cgroup before joining output readers, waits at most two
  seconds for empty membership and removes only the newly created group.
  Explicit cleanup errors refuse commit. Drop retries best-effort cleanup on
  early errors. Nonblocking bounded pipe readers cannot hang on inherited open
  writers after teardown; late stderr overflow also refuses the result.
- Unit coverage exercises missing/invalid delegation, nonsymlink control opens,
  missing controls, ceiling readback mismatch, strict bounded populated-state
  parsing, membership-write failure, pipe overflow, held-open writers and I/O
  failure. Two explicit opt-in kernel tests cover attachment/readback, lock
  contention, group kill/removal, and reduced-limit OOM/task pressure. They are
  ignored by default, not claimed as passing kernel evidence.

## Proven in this run

Validation results are recorded after the final source edits below. Tests run
with `env -i`, private HOME, mode-0700 XDG_RUNTIME_DIR and ONE_MK_HOME under the
worktree, one CARGO_TARGET_DIR, `-j 2`, no owner WAYLAND_DISPLAY or D-Bus address.
No `cargo fmt` runs. No GNOME harness, renderer, importer worker or daemon is
manually launched. Existing daemon integration tests select the null backend
with discovery/notifications disabled.

- Final `cargo test --offline -j 2 --workspace`: **passed**, 1,158 passed,
  0 failed, 16 ignored across 39 result records (including doc-test records).
  Two ignored tests are the new explicit-delegation kernel tests. Conditional
  audited-runtime tests return without qualification when no runtime is supplied;
  their `ok` result is not a native decoder/sandbox pass.
- Final `cargo clippy --offline -j 2 --workspace --all-targets -- -D warnings`:
  **passed**. `git diff --check`: passed. No formatter was invoked.
- Importer `cargo test --offline -j 2 -p one-mk-wall-import --features test-faults`:
  **passed**, 30 passed / 0 failed / 2 ignored. Importer `cargo clippy --offline
  -j 2 -p one-mk-wall-import --all-targets --all-features -- -D warnings`:
  **passed**. These additional builds compile-check fault-injection paths; the
  runtime-dependent checkpoint/storage tests still need the explicit lab runtime.
- The first sandboxed workspace attempt could not create loopback sockets
  (`EPERM`); subsequent shared-test mutex poisoning caused cascading failures.
  The approved reruns retained the same private environment but allowed test
  loopback sockets, and passed. No unrelated test/code change hid that failure.
  The fault-feature pipe test likewise needed approved Unix socket-pair access;
  its rerun passed without changing the test.

Implementation commit: `4be1198` (`Require delegated cgroup limits for wallpaper
imports`). Documentation is committed separately, with M1 still marked incomplete.

To reproduce, use a clean environment with PATH plus throwaway HOME,
XDG_RUNTIME_DIR (0700), ONE_MK_HOME and a single worktree CARGO_TARGET_DIR.
Supply only the toolchain CARGO_HOME/RUSTUP_HOME, not owner display/bus variables.
The native tests require a separately provisioned private delegation and are
deliberately **not run here**:

```text
ONE_MK_WALL_TEST_CGROUP_ROOT=<explicit-private-delegation>
cargo test --offline -j 2 -p one-mk-wall-import --lib cgroup::tests::delegated_kernel -- --ignored --test-threads=1
```

The pressure test additionally needs `/usr/bin/python3`; it lowers its new
private group's memory limit to 32 MiB, attempts a bounded 96 MiB allocation,
then tests a four-task limit. It checks kernel OOM/task event counters rather
than treating an arbitrary child failure as resource-enforcement evidence.
Do not point it at the owner's live session delegation.

## Not proven / remaining

- The host mounts `/sys/fs/cgroup` read-only. No live cgroup was provisioned or
  mutated, and no owner service-manager/session hierarchy was used. The new
  kernel tests and the audited-runtime binary import matrix are not run here.
  Successful native setup, actual memory/task pressure, concurrent native
  cleanup and repeated cancellation still require a controlled private lab.
- The ceiling applies to cgroup-charged worker memory, not parent snapshot page
  cache, inherited/shared charges, GPU memory or all imports under unrelated
  delegations. Provisioning a machine-wide one-worker service/queue is not done.
- Parent SIGKILL cannot run Rust Drop. Existing bubblewrap parent-death and PID
  namespace teardown remain the crash boundary, but have not been requalified
  with the cgroup path. Empty group directories can remain after parent death;
  no potentially destructive stale-group recovery is added. Do not infer
  crash-proof cleanup from the normal teardown tests.
- Daemon-to-renderer owner-authenticated presentation plans, consent-revision
  subscriptions and revocation-driven presentation teardown are **not done**.
  Production authorization stays false. No test-only override is promoted.
- Imported local media playback through the production renderer, one canvas
  across monitors, lock/unlock, suspend, hot-plug and daemon stop/crash desktop
  restoration are **not done/proven** end to end. Reserved absent-computer canvas
  geometry and the VP9/AV1 WebM + GIF/APNG/animated-WebP admission policy are
  unchanged. No third-party wallpaper-project source is copied.
- No new Mutter run means no claim of zero new warnings versus baseline.
  Previous spike/component evidence is not transferred to this slice. Physical
  hardware, power, native Windows/macOS sandboxes, normalization and signed
runtime packaging remain acceptance gates.

## Handoff cleanup

All cargo/test command sessions have exited and been reaped. No persistent
server, daemon, Mutter session or renderer was started by this slice. The single
build target `.m1-target`, fixture-only `target` tree created by existing tests,
and private `.m1-lab` HOME/runtime/config/log tree were removed and absence
checked. No manual `/tmp` scratch was created. No owner configuration, clipboard,
Wayland socket or D-Bus session was accessed, installed into or changed.

M1 stays incomplete; retain the existing low-confidence 16–26 engineer-week
remaining estimate. Next slice requires a qualified isolated delegated-cgroup
lab before claiming importer resource acceptance, then authenticated daemon
presentation integration before any production renderer can be enabled.
