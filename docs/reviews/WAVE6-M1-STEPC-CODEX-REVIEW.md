# Step C adversarial review — October 31, 2026

## Scope and disposition

Reviewed exactly `git diff 7691369..509eb17`, including the lifecycle harness
and its checked-in evidence. The requested merge started at `7691369` and
needed a test-only conflict resolution in `playback.rs`: both the base's
absolute worker-header deadline tests and Step C's holding test were retained.
Merge commit: `55d81f3`. The merge also preserves the base's odd-height I420 and
qualification-measurement review fixes rather than reverting them to the
older parent of Claude's branch.

Two correctness defects fixed in small commits. Historical qualification is
useful evidence, not proof of every interleaving. Idle-work and terminal
SIGKILL reclamation limitations remain; this is not an unconditional Step C
acceptance or a production-authorization approval.

## Findings

### P1 — exact snapshot diff drops Step B security fixes (preserved by merge)

The comparison requested by the review is not just an incremental Step C
patch: `509eb17` branched before some fixes present in `7691369`. In that
exact diff, `start` switches back from the absolute-deadline `read_header`
to unbounded `StreamHeader::read`, the deadline helper/tests disappear, and
the odd-height I420 plane fix and its crop regression disappear. Applying
that snapshot as a replacement would regress bounded shutdown on hostile
worker headers and odd-height frame correctness. The three-way merge retains
these base fixes; I preserved both sides of the test conflict and checked the
merged call site still uses the three-second absolute header deadline.
The preserved deadline tests pass. Do not replace the reviewed merge with a
checkout of the older-based `509eb17` snapshot.

### P1 — coverage could reveal stale slices during layout debounce (fixed)

`Bridge` hides the layer synchronously on `monitors-changed`, but delays the
layout/serial update by 100 ms. During that interval `_renderer.fresh` still
describes the old layout. A coverage callback (restacking, workspace change,
state update, overview/session event) unconditionally recomputed visibility
from that flag and could reveal the old slices on the new monitor layout.
The later serial check in `_damaged` cannot protect this earlier interval.

Fix `72c6279`: both `_coverage` and `_rebuild` refuse to reveal the layer while
the existing named `layout` source is pending. The debounced callback removes
that source before settling the layout, so unchanged geometry can still
reveal its valid window without another commit. No new timers or protocols.

Regression: `test_extension_lifecycle.js` executes the actual extension class
with mocked Shell objects and invokes coverage while a layout source is
pending. Before the fix it failed with “coverage re-revealed the old slices
during monitor-layout debounce”; afterwards it passes and also verifies that
a settled fresh layout remains revealable. No GNOME session or bus is used.

### P1 — a missed long-frame deadline could disable progress detection forever (fixed)

`step` announced Holding on a long-frame commit. It only announced Moving
after successfully reading/composing/committing a subsequent short frame.
At the long frame's deadline an empty frame channel or busy compositor buffers
returned without leaving Holding. A stalled decoder/reader could therefore
leave a visibly frozen layer with no Shell damage watchdog indefinitely.
The daemon heartbeat checks authority, not decoder progress, so it cannot
detect this failure. Consecutive long frames also lacked an explicit
deadline-to-next-commit progress window.

Fix `fc2ab06`: expire a finite hold at its existing frame deadline and enqueue
Moving **before** checking frame availability or free buffers. Successful
commit of another long frame re-enters Holding; a missed commit now starts
the existing Shell watchdog instead of granting unbounded immunity. A still
has no deadline and remains held without a presentation timer.

Regression: `long_frame_hold_expires_before_waiting_for_the_next_frame` checks
before/at/after-deadline behavior, one-shot expiry and indefinite still holds.
The helper is called on the actual scheduling path before either starvation
return. All five playback unit tests pass, including the preserved header
deadline tests. This is unit-level deadline coverage, not a native decoder
stall or compositor-buffer starvation qualification.

### P2 — “no work while holding/paused” is not satisfied (not changed)

The presentation session still polls authority every two seconds. Plan-paused
sessions keep that heartbeat too. The actual holding evidence records **27
main-thread context switches and zero presenter-thread switches in 12 s**.
Only the presenter and the Shell damage watchdog were demonstrated idle;
the entire renderer was not. Animated long frames also have bounded decoder
prefetch, not proof of a process-wide no-work interval.

This is already acknowledged in the report's “Not proven / not done” section.
Removing the heartbeat changes the wedged-authority detection contract and
is explicitly deferred to Step D. I did not silently remove that safety net
or redesign authorization in a lifecycle review. Under the requested strict
zero-work interpretation, this remains an acceptance gap. Lock/suspend
Shell-chosen stops differ: the renderer exits, so its heartbeat disappears.

### P2 — terminal renderer SIGKILL leaves an empty cgroup (not redesigned)

Reclamation is performed by the **next renderer**, not a surviving janitor.
If restart is disarmed, the extension is disabled, or the crash cap latches,
there may be no next renderer. A renderer killed before `Group::finish` then
leaves its empty `play-*` directory. The evidence explicitly records ten
`stale_empty_groups_after_bare_sigkill` observations, each equal to one.
This is an empty kernel-group leak, not evidence of a live escaped worker.

`teardown(renderer-kill)` disables authorization before the kill, checks that
workers died and stale groups are empty, then calls `reclaim_check()`. That
helper deliberately starts another daemon/renderer to sweep the group before
the final restoration/no-leak check. The crash-cap proof also invokes it.
Thus final empty groups do not establish autonomous cleanup after terminal
SIGKILL. A persistent cleanup owner requires lifecycle/authorization design
beyond this narrow fix; the limitation must remain visible, not be called
immediate or unconditional restoration/reclamation.

### P2 — qualification counts and restoration claims need narrower wording (documented)

The final JSON does contain 100 sequence records, seed `20261031`, and
`passed: true`. However its `events`/`event_counts` aggregate **directed proofs
as well as randomized sequences**. For example, the 100 records contain ten
`stop` teardowns, whereas the aggregate says nineteen. The published counts
119 layouts, 59 locks, etc. are run totals, not counts within the randomized
sequences alone. Each sequence selects one item, 2–4 weighted events and one
weighted teardown; repeated renderer kills are converted to locks, and an
unfit layout can cause an additional fitting-layout event. This is seeded,
constrained coverage, not arbitrary exhaustive event ordering.

Restoration compares a sampled screenshot's wallpaper body with a generated
private native baseline (mean error <0.5; recorded 0.0), verifies the private
picture URI/options, and checks marked processes/groups/bridge state. Daemon
teardowns call `disarm()` before this final check. Bare kills may also perform
the explicit restart/reclamation described above. Shell restart re-applies
the memory-backed wallpaper setting before checking it. None of these are
measurements of the owner's real desktop or continuous absence of flashes.
Layout probes take discrete state/screenshot samples; 150 ms burst steps
allow restarts and only require convergence. Those samples missed the
100 ms coverage race fixed here.

Four source hashes embedded in `result.json` match their exact files at
`509eb17` (extension, lifecycle script, fake logind, isolated session script).
Attempts 2 and 3 have the same binary hashes as the final run. Attempt 2
contains 100 records and failed the evolution-addressbook warning gate;
attempt 3 contains 97 and failed `seq097`'s 3.72 s layout bound. This supports
the documented changes to qualification rules, but hashes/logs are not a new
execution or independently reproducible proof of runtime isolation.

## Additional paths inspected

- Window adoption still requires `Meta.WaylandClient.owns_window`; changing
  damage/holding behavior did not replace this with title, PID-only, or
  geometry-only trust. D-Bus caller binding also pins UID, PID and client
  identity across asynchronous credential lookup. No hostile-window bypass
  was demonstrated in this review.
- Old-layout damage cannot mark an adopted old-serial window fresh. Layout
  changes invalidate admission and fetch an authenticated new plan; playback
  reports carry generations and replaced/stopped reports are ignored.
- `_stop` clears the current client before signaling it; `terminate` deduplicates
  per subprocess and its escalation survives extension disable. Reap callbacks
  cancel the escalation. Planned pause stops refund crash attempts; crashes
  still count. Disable/owner loss hide clones rather than rewriting the user's
  wallpaper settings. These checks do not establish real suspend timing.
- SIGTERM drops the session/presenter, kills/reaps the worker and removes its
  group on the normal path. SIGKILL cannot run that destructor. Reclamation
  checks the trusted cgroup-v2 root and exclusive lock; I did not touch any
  owner/agent cgroup to exercise it.

## Validation and isolation

Only clean environments with throwaway HOME, mode-0700 XDG_RUNTIME_DIR,
ONE_MK_HOME and one worktree `.review-target` were used. No display address or
real session/system-bus address was inherited. No daemon, GNOME Shell, X
server, clipboard client, live-session command or native worker was manually
started. No delegated cgroup was supplied/created, so native lifecycle and
cgroup conclusions are static/evidence audits, not newly qualified runs.
The 100-sequence harness was not rerun. No formatter was run.

Targeted GJS regression: red before fix, green after. Targeted Rust playback
tests: initial sandbox run hit `UnixStream::pair` EPERM; the approved clean-env
rerun outside that restriction passed 5/5. The new deadline test passed; the
existing stream tests also passed without changing them.

Final gates, each run exactly once after the fixes, both with the laboratory
features enabled so the changed code was included:

- `cargo test --offline -j 2 --workspace --features one-mk/wallpaper-m1-lab,one-mk-wall/m1-lab`:
  **1,229 passed, 0 failed, 17 ignored**, 43 result records. Conditional
  runtime-dependent tests are not native sandbox/decoder qualification.
- `cargo clippy --offline -j 2 --workspace --all-targets --features one-mk/wallpaper-m1-lab,one-mk-wall/m1-lab -- -D warnings`:
  **passed**.
- `git diff --check`: **passed**.

An outside-sandbox `/proc` executable-path scan found **zero surviving
processes from this worktree's review target**. The sole 8.7 GiB target and
throwaway environment/log directory were deleted after recording the results.
No manual `/tmp` scratch was created, no scope/cgroup was borrowed, and no
owner desktop/session/configuration was touched.
