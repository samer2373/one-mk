# Wave 6 M1 implementation slices — NOT M1 completion

## Step C adversarial review addendum

The historical run below predates the fixes and limitations recorded in
[`WAVE6-M1-STEPC-CODEX-REVIEW.md`](WAVE6-M1-STEPC-CODEX-REVIEW.md).
Review found and fixed a stale-slice reveal during layout debounce and an
unbounded long-frame hold after decoder starvation. The run's event totals
include directed proofs, not just its 100 randomized sequences. Final clean
restoration checks may follow lab disarming and, after a bare renderer kill,
an explicitly started reclamation renderer. Zero presenter wakeups is not
zero whole-renderer work. Treat “never old slicing” as sampled historical
observations, not a guarantee over every race; these fixes have targeted
regressions, not a rerun of native lifecycle qualification.

## Latest slice: step C reached in the private laboratory (D not)

October 31, 2026 (Asia/Amman). Branch `worktree-agent-ad820fc757181bd93` on top
of `500f237` (steps A, B and review fixes). **Step C of 6D-M1 (lifecycle and
restoration) is done and proven in the private GNOME 50.1 harness with real
playback; M1 is not complete** (step D, production authorization, stays false).
The 16–26 engineer-week remaining estimate is not reduced by laboratory steps.

**Done.** Extension: cooperative stops (SIGTERM, SIGKILL after 2 s, escalation
survives disable); suspend via logind `PrepareForSleep` (pause reason 8 until
resume, prompt restart on wake, like unlock); bridge state 4 *holding* (stills
and frames ≥2 s announce that no damage is due, so neither the renderer's 2 s
keepalive commits nor the Shell's damage watchdog run); windows adopted under
an older layout serial are never revealed again and the layer hides without a
fresh window; Shell-chosen stops no longer consume the three-per-minute restart
cap (crashes still do); the first healthy frame ends the start deadline; stop
causes recorded. Renderer (`m1-lab`): clean SIGTERM teardown (worker, cgroup,
surface), Holding/Playing reports per playback generation, spurious Wayland
`EAGAIN` no longer fatal, stale playback groups reclaimed at start. Publisher/
Gate: a local layout outside the committed canvas is published paused (static)
and admitted; a playing plan must still fit. Harness: stand-in logind on the
private system bus, explicit layouts, in-place session restart, own delegated
transient scope, `qualify-m1-lifecycle.py` with a no-component control phase.

**Proven** by `qualify-m1-lifecycle.py` ([evidence](../spikes/gnome/m1-lifecycle-evidence/README.md)),
three virtual monitors, eight locally generated imported items:
(1) add/remove/scale 2/scale 1.25/reposition/rotation: same renderer re-plans,
verified exact slices revealed in median 0.57 s, max 1.58 s over 111 changes;
never old slicing on screen; a layout outside the canvas is static and recovers;
(2) lock: renderer exits by SIGTERM (median 97 ms, max 208 ms), nothing of ours
exists while locked, unlock shows verified frames (max 2.6 s), six cycles a
minute leave the cap untouched; (3) simulated suspend with and without
lock-on-suspend: exit before sleep (max 49 ms), user wallpaper while asleep,
fresh verified frames after wake (max 1.8 s), no worker left; (4) after daemon
stop/SIGTERM/SIGKILL, revoke, disable, bare renderer SIGKILL, extension disable,
uninstall and Shell SIGKILL + restart, the screenshot equals the user's own
generated wallpaper (mean error 0.0), `picture-uri` untouched, no process or
cgroup of ours left (a bare renderer SIGKILL leaves one empty group until the
next renderer reclaims it); (5) a still shows 12 s in state 4 with zero
presenter wakeups and no Shell watchdog; (6) 100 randomized sequences (seed
20261031: 119 layout changes, 23 bursts, 59 locks, 68 suspends, 21 renderer
kills, 33 pause/play, 25 extension cycles, every teardown kind, 16 Shell
restarts) all ended verified or correctly static with nothing left; (7) zero
new Mutter/Shell warning records versus the M1a-2 baseline across 18 sessions,
with harness-caused identities measured separately in a control phase with
nothing of ours enabled. Bonus: the importer's two native cgroup kernel tests
passed against a child of the private delegation.

**Not proven / not done.** Step D: production authorization and its channel
(the laboratory re-authorizes each new bridge instance by private Eval; after a
daemon stop the authorized lab Shell retries the renderer, which fails and
counts toward the cap — production must withdraw authorization instead).
Suspend is the logind signal only (no real sleep, frozen processes, GPU/DRM
resume); lock has no real GDM/authentication; virtual monitors only (no
physical outputs, mixed scales, HiDPI sampling or content rotation); an unfit
layout pauses all local outputs (per-output static needs 6D-M2 wire changes).
The still keepalive is gone, but a 0.5 Hz authenticated heartbeat to the daemon
remains in the renderer by design (wedged-daemon bound; zbus does not drop
`Changed` signals); removing it is a step D decision. "Shell restart" is SIGKILL
plus a fresh private session with the wallpaper setting re-applied by the
harness. One machine; no power or long-duration data. No Muro/Aura/Hanabi code
was consulted or copied.

**Validation.** Clean env (`env -i`, throwaway HOME, mode-0700 XDG_RUNTIME_DIR,
ONE_MK_HOME, one worktree CARGO_TARGET_DIR, `-j 2`, offline): `cargo test
--workspace` **1,212 passed, 0 failed, 17 ignored** (43 result records);
`cargo clippy --workspace --all-targets -- -D warnings` passed, also with
`--features one-mk-wall/gnome-bridge` and with
`--features one-mk/wallpaper-m1-lab,one-mk-wall/m1-lab`; `one-mk-wall` lab unit
tests 7 passed; daemon lab tests 13 passed; warning-gate Python tests passed;
`git diff --check` clean. No formatter ran. The qualifying harness run was the
third 100-sequence attempt; attempts 1–3 failed only on harness rules (lossy
frame-identification margin; an evolution-addressbook record at Shell kills;
a bound that included verification time), recorded in the evidence; no product
code changed between attempt 1 and the qualifying run.

**Cleanup.** The harness finalizer stopped the private session (no remaining
laboratory process or playback group in `result.json`); the transient scope,
its cgroups, the trusted-runtime directory, `/tmp` scratch and the worktree
build target were removed after the evidence was copied. No owner desktop, X
server, clipboard, real session bus or `~/.config/one-mk` was touched.

## Previous slice: step B reached in the private laboratory (C–D not)

October 31, 2026 (Asia/Amman). Branch `worktree-agent-a09a60f8d1fd50bec` on top
of `2b5bf48` (step A). **Step B of 6D-M1 is done and proven with real
processes and real pixels; M1 is not complete.** The 16–26 engineer-week
remaining estimate is not reduced by this laboratory step.

**Done.** Importer: still PNG admission (one frame, no duration), per-frame
container timeline, and a `--play` worker launched only by
`sandbox::spawn_playback` (same sandbox; no CPU-seconds ceiling; delegated
cgroup with at most two CPUs when the CPU controller is delegated (otherwise
no aggregate CPU quota); stale playback groups reclaimed under the
delegation lock). Images loop through the image crate; VP9/AV1 run through
in-process GStreamer from the pinned runtime (preroll for colorimetry, I420
out, flushing-seek loop, no new process); a still sends one frame and exits.
Renderer (`m1-lab`): resolves and re-hashes the plan's media in the private
import library, starts the worker, and a presenter thread draws one xdg surface
(two `wl_shm` buffers, first buffer only after the configure ack) with an exact
centre-anchored cover map per monitor slice; I420 is converted only where
shown. Pause reports Paused first, then destroys worker and surface. Extension:
the first damage (before buffer geometry exists) no longer stops the renderer.
Artifact gate: the release renderer tree may not contain laboratory playback.

**Proven** in the private GNOME 50.1 harness by `qualify-m1-playback.py`
([evidence](../spikes/gnome/m1-playback-evidence/README.md)): all seven
locally generated items imported by the real importer; GIF/APNG/still exact on
every pixel below the top bar on both monitors, WebP within one level (the
image crate's WebP compositing, measured independently), VP9/AV1 >= 34 dB PSNR
at sampled source-pixel/block centres (not a full-output PSNR measurement);
same frame on both monitors, reserved absent slice never drawn; real-time
frame progress and wrap; 28 loop commit seams within 0.4 ms of nominal (not
compositor presentation feedback); still stable
4.5 s with no decoder; plan pause → native background, worker and surface gone,
0 CPU ticks, resume; renderer SIGKILL kills its workers and the next playback
reclaims the group; 1080p30 VP9 renderer 13.1 % + worker 42.6 % (+ Shell 8.5 %)
of one core, renderer RSS 38.8 MiB, worker cgroup 67 MiB, 0 late; 100 real
playback cycles (7 items x 5 teardowns, exit <= 84.8 ms) with no leftover
worker, group or process; zero new warning records versus the M1a-2 baseline.
A native test also runs the worker in the real sandbox and delegation
(images, still, VP9/AV1 loop bytes, hostile input, cleanup).

**Not proven / not done.** Production authorization stays false (D). No
layout change/hot-plug, lock/unlock, suspend/resume or restoration evidence
(C). Cover fit only, CPU nearest sampling, no GPU/HiDPI/rotation/physical
display, no second computer or cross-computer clock (resume restarts at frame
0). Stills and frames longer than 2 s are re-committed every 2 s because the v1
bridge cannot announce frame deadlines (design target not met). One machine,
synthetic software-decoded clip; design CPU budgets not met by software decode
and not claimed; no power/long-duration data. The laboratory cgroup delegation
was a directory created for this run under the user's own (T3 Code agents)
systemd slice, then removed; production provisioning is undesigned. No
Muro/Aura/Hanabi code was consulted or copied.

**Validation.** Clean env (`env -i`, throwaway HOME, mode-0700
XDG_RUNTIME_DIR, ONE_MK_HOME, one worktree CARGO_TARGET_DIR, `-j 2`, offline):
`cargo test --workspace` **1,197 passed, 0 failed, 17 ignored** (42 result
records); `cargo clippy --workspace --all-targets -- -D warnings` passed, also
with `--features one-mk-wall/gnome-bridge` and with
`--features one-mk/wallpaper-m1-lab,one-mk-wall/m1-lab`; the native playback
worker test passed against the lab runtime and delegation; the artifact
checker passed with the 32-file lab runtime and refused the laboratory
renderer tree; warning-gate Python tests passed; whitespace check clean. No
formatter ran.

**Cleanup.** The harness finalizer stopped the private session; no laboratory
process or playback group remained (`result.json`). The `/tmp` laboratory and
scratch directories, the worktree build target, the trusted runtime directory
and the laboratory cgroup delegation were deleted after evidence was copied.
No owner desktop, X server, clipboard, real session bus or `~/.config/one-mk`
was touched.

## Previous slice: step A reached in the private laboratory (B–D not)

Adversarial follow-up: [Codex step A review](WAVE6-M1-STEPA-CODEX-REVIEW.md)
records the coalesced-consent revocation fix, release-feature guard hardening,
failure-path harness cleanup, regression results and historical evidence limits. The recorded
GNOME run predates those fixes and is not a rerun of the reviewed tree.

October 31, 2026 (Asia/Amman). Branch `worktree-agent-a69b251f0c19489d7`, on top
of the stopped Codex WIP `2145bea` (merged; its Gate change that keeps the admitted
plan on refusal is kept, its inherited-descriptor bootstrap and daemon-side
fault file were replaced). Commits `764545d` (I/O-free session core), `f9604af`
(daemon authority, renderer session, harness), `15a8b31` (artifact gate). **Step A
of 6D-M1 is done and proven with real processes; M1 is not complete.** The 16–26
engineer-week remaining estimate is not reduced by this laboratory step.

**Done.** `one_mk_wall_sync::presentation_auth`: a private authority record
(per-run key, generation, PID, unique bus name), keyed BLAKE3 request/reply tags
binding both bus names, a fresh nonce and every field, a daemon-side publisher
(sequence advances only on change; per-monitor slices on the frozen shared canvas)
and the renderer session that admits plans through `Gate::accept_encoded`, keeps
the admitted plan on refusal and tears down on forged tags, revoked/replaced
consent, owner loss, invalid Shell layout/pause or 16 refusals in a row. The daemon
(feature `wallpaper-m1-lab`, null backend/no discovery/no notify/no clipboard only)
serves `org.onemk.WallpaperAuthority1.Plan`, follows live `[wallpaper]` consent
(the real `one-mk wallpaper consent revoke` / `disable` CLI) and signals changes to
the pinned renderer at once. The renderer (feature `m1-lab`, `--present=gnome
--m1-lab`) authenticates the Shell bridge first, then the record's authority
(owner PID/UID checked after subscribing to owner changes) and exits on every
teardown. Design and threat-model limits: [session design](../spikes/gnome/presentation-wire.md).

**Proven** in the private GNOME 50.1 harness by `qualify-m1-presentation.py`
([evidence](../spikes/gnome/m1-presentation-evidence/README.md)): (1) absent,
stale (SIGKILL leftover and replayed), stale-key, wrong-key, wrong-PID and impostor
authorities refused with nothing admitted; unauthenticated callers refused by the
daemon; (2) the daemon's plan (media hash, two monitor slices on a 5760-wide canvas
with a reserved absent third, pause, consent revision) admitted; replayed,
stale-sequence, stale-revision, 4097-byte, 64 KiB-envelope and malformed plans
refused with the previous plan intact in the same process; fresh plan required
after a Shell layout change; a forged mid-session reply ends the renderer;
(3) consent revoke/disable and daemon `one-mk stop`/SIGTERM/SIGKILL end the
renderer in at most 86 ms (2 s bound asserted), Shell restarts refuse too, every
PID reaped, no leftover laboratory process; (4) 100 real-process start/stop cycles
rotating those five teardowns, zero new warning records against the M1a-2 baseline.

**Not proven / not done.** No surface or decoder exists: admitted/active is intent,
so there is no media playback, presented-frame, restoration, lock/unlock,
suspend/resume or hot-plug surface evidence (steps B/C). Production authorization
stays false (D): both features are off in release builds, plain `--present=gnome`
refuses and the artifact gate now rejects a daemon linking the authority. The
production credential bootstrap and packaging are undesigned; "crash" was
exercised as SIGKILL (no core-dumping signals, to keep the host coredump handler
out); same-UID readers of the runtime record are outside the threat model. No
physical GPU/HiDPI, Windows/macOS or multi-computer evidence. No Muro/Aura/Hanabi
code was consulted or copied.

**Validation.** With `env -i` (PATH, throwaway HOME, mode-0700
XDG_RUNTIME_DIR, ONE_MK_HOME, toolchain homes, one worktree CARGO_TARGET_DIR),
`-j 2`, offline: `cargo test --workspace` **1,190 passed, 0 failed, 17 ignored**
(42 result records including doc tests; +11 `presentation_auth` tests);
`cargo clippy --workspace --all-targets -- -D warnings` passed, also with
`--features one-mk-wall/gnome-bridge` and with
`--features one-mk/wallpaper-m1-lab,one-mk-wall/m1-lab`; `cargo test -p one-mk
--features wallpaper-m1-lab --bin one-mk wallpaper` 9 passed (3 laboratory
authority tests); warning-gate Python tests and the artifact checker self-test
passed; `git diff --check` clean. The private-harness run used the binaries pinned
in the evidence `source-sha256.txt`.

**Cleanup.** The harness finalizer stopped the private session; the host process
scan found no laboratory process. Both `/tmp` laboratory directories, the worktree
build target and the private `.lab` tree were deleted after evidence was copied.
No owner desktop, X server, clipboard, real session bus or `~/.config/one-mk` was
touched; no formatter ran.

## Previous slice: encoded-plan precursor — none of A–D reached

October 31, 2026 (Asia/Amman). Implementation commit `f93761a` adds a bounded
presentation payload codec and `Gate::accept_encoded`. **The requested real
daemon → renderer integration is not delivered.** This is another preparatory
change, not completion of stage A or 6D-M1. The 16–26 engineer-week remaining
estimate is unchanged.

**Done:** an independently versioned, canonical ≤4096-byte payload carries the
existing plan without credentials or paths. Counts, UTF-8 screen lengths, flags,
reserved bytes, truncation and trailing data are checked before bounded allocation.
Encoded admission clears existing intent, checks pinned caller identity and
independent live consent, then applies the original semantic Gate. Invalid input
cannot reset the replay floor; changed/removed consent remains terminal. Reserved
canvas space is preserved. Ten additional tests cover these cases, deterministic
arbitrary-byte inputs and every single-byte mutation of a valid plan. The exact
[wire contract and integration boundary](../spikes/gnome/presentation-wire.md)
make clear that structurally decoded data is not authorization.

**Not done / stop boundary:** no production daemon, bridge or renderer calls the
new API. The existing bridge XML has deliberately not been extended with media
selection or paths. Trusted real-daemon authority bootstrap, independently sourced
live consent snapshots/subscriptions, real plan publication, renderer teardown
on daemon loss/revocation and actual process-exit wiring remain unimplemented.
Consequently **A is not reached; B and C were not attempted; D stays false**.
There is no real imported-media playback, presented-frame evidence, 100-cycle
real-playback gate, restoration or native lifecycle acceptance in this slice.
No Muro/Aura/Hanabi code was copied. Production authorization stays false.

**Validation:** clean-env `cargo test --offline -j 2 --workspace` passed:
**1,179 passed, 0 failed, 17 ignored**, 41 result records including doc tests.
Clean-env `cargo clippy --offline -j 2 --workspace --all-targets -- -D warnings`
passed, as did the `one-mk-wall/gnome-bridge` feature variant. The warning gate's
six Python tests passed. No formatter ran. The first restricted workspace run
failed on socket creation; the approved retries exposed group-writable ancestor
guards and an overlong Unix socket fixture path. A fresh short mode-0700 `/tmp`
root resolved those fixture-environment failures without changing unrelated code.
All attempts are distinguished in the retained validation evidence.

Private bridge regression and final cleanup results are recorded in
[encoded-admission evidence](../spikes/gnome/encoded-admission-evidence/README.md).
The approved isolated GNOME 50.1 run passed 100 authenticated paused-fixture
cycles, every exact child PID/handle reaped, unchanged private background URI,
and zero new baseline warnings. This is **not** the requested real-playback gate.
The private session/copied extensions, single worktree target and both exact
`/tmp` scratch roots were removed and absence checked after evidence retention.
The host marker scan found zero matching test/lab processes; four unreadable
same-user service environments had process start times predating this run and
were not touched. The working desktop/session bus/config were never accessed.

## Previous slice: presentation admission precursor, not playback

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
