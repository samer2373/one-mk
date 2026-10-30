# 6D-M1a-4 resolution: bridge stability fixed; product M1 remains incomplete

Date: October 30–31, 2026 (Asia/Amman). Branch
`t3/stabilize-wallpaper-bridge`, reset to local `wave5` at `7684a99`
(`Merge wallpaper lifecycle gate work (M1a-3) into wave5`). Only the isolated
headless Mutter harness ran; the owner's desktop, running daemon and user
extension directory were not used or changed. No install/enable is automatic.

## Current decision and requested-item status

**The intermittent dispatch and slow healthy-unlock recovery are resolved in
the spike. M1 is still incomplete, not a shipping acceptance.**
[M1a-4 evidence and reproduction](gnome/m1a4-evidence/README.md) contains original
failures, all six final stress runs, full Shell/GDB logs, lifecycle/security/idle
reports, validation and hashes. Earlier STOP findings below are retained as
history, not current claims that cycle 75 is still failing.

| Requested item | Current status |
|---|---|
| 1: UnknownMethod cause, lifecycle, readiness, bounded renderer retry | Implemented in spike; GC reproduction catches stale introspection-cache lookup; five consecutive 100-cycle sessions plus 500 cycles pass, zero new warnings |
| 1: historical cycle-19 Shell loss | Investigated, **not root-caused**; old signal/core absent, PID-filtered journal empty; no recurrence in final sessions |
| 2: healthy unlock versus crash retry | Implemented; 93.138 ms notification-to-damage / 451.024 ms request-to-damage in initial final matrix, versus previous 4092.193 ms notification-to-damage; crash 1/2/4 s cap retained |
| 3: production extension, §4 bridge, doctor, qualification guard, extension CLI | **Undelivered**, not made safe by spike stability. No production source/metadata or installer added; no change to `main.rs` importer dispatch |
| 4: final lifecycle and power matrix | Passed for the final **spike**, including the nine-layout repeat; product/native/physical matrix still unqualified |
| 5: resolution / feature plan / estimate | Updated; remaining M1 16–26 engineer-weeks, total 74–118, retained with low confidence rather than reduced on spike results |

## Cause and explicit lifecycle

Per-enable `wrapJSObject` builds an additional interface lookup cache in the
installed GJS 1.88.0 override, but that extra cache build is not balanced when
the interface dies. GLib's global cache borrows the native interface pointer and
method storage. GC can leave stale entries; subsequent allocation reuse explains
why the real `Hello` remains in introspection while cached method lookup is
null. The latter disagreement is directly captured at cycle 79; the actual
renderer `UnknownMethod` reproduces at cycle 22. Pointer reuse is source-level
diagnosis, not an independently captured allocator trace. Shell stays alive in
both reproductions. This distinguishes the bug from a name-handoff delay.

The fix retains module-level interface info, constructs the exported object
directly instead of `wrapJSObject`, keeps the object strongly referenced and
exports once per enable. Disable invalidates callbacks, stops/reaps the child,
unexports, then relinquishes the bus name. A bounded 500 ms self-dispatch
handshake after name acquisition gates all starts. Name loss and readiness
failure fail closed. The renderer retries only transient named dispatch errors,
at most ten 100 ms attempts with 25 ms between attempts, never an endless loop.

A deliberate teardown of a healthy renderer grants one immediate resume after
fresh enable/readiness. It cannot clear crash history or an exhausted latch.
Crashes still use 1/2/4 seconds and at most three automatic starts in 60 seconds.
The four-crash test verifies static/no-child/no-restart-source after exhaustion,
including disable/re-enable. This spike latch is **not** product doctor support.

## Measured lifecycle and remaining limitations

Latest final nine-layout matrix (`final-matrix-nine-layouts.json`),
compositor-side timestamps; initial final matrix retained for comparison:

| Scenario | Measured final-code spike result |
|---|---|
| Startup, three outputs | 230.784 ms collector-to-first damage (initial matrix 224.309 ms) |
| Monitor unplug / replug | 39.562 / 30.660 ms Shell layout-notification-to-first new-window damage (initial 26.543 / 30.495 ms) |
| Renderer SIGKILL | 2083.002 ms request-to-fresh replacement damage (second crash backoff) |
| Hung renderer | Static fallback observed at 2560.59 ms; child reaped, fresh replacement follows |
| Lock / unlock | Child removed while locked; notification-to-fresh damage 109.128 ms; request-to-damage 452.663 ms (initial 93.138 / 451.024 ms) |
| Crash cap | Recovery observations 1291.13 / 2157.59 / 4225.01 ms; fourth death exhausts, pid null and restart source 0, latch survives re-enable |
| Outputs/scales | Initial seven-layout matrix passes; final repeat covers every 1/2/3-output × 1/2/1.25-scale pair with correct clone count and extents |
| Desktop icons | DING enabled, desktop focus retained; no managed renderer window after disable |
| Repeat teardown | Five consecutive 100-cycle runs + 500-cycle run, forced GC throughout; further 100 cycles in each full matrix; zero extension actor/clone/managed-window leaks |
| New warnings | 0 versus unchanged committed M1a-2 disabled baseline, full stack-aware comparison |

These are damage/readiness timings, **not physical scanout**, whole-visible-gap
proof or real-GDM hardware testing. The virtual layout checks do not qualify
mixed-DPI hardware, rotation or the native GPU buffer limits. The nine-layout
repeat is separately retained, not used to replace a failed run.

Final matched strace/CPU windows (`final-idle.json`, three seconds each):

| State | Renderer CPU, one core | Shell CPU, one core | Blocking-poll completions/s | Timer returns/s |
|---|---:|---:|---:|---:|
| Playing, three outputs | 59.66% | 9.00% | 29.994510 | 125.643671 |
| Paused by overview, two outputs | 0.00% | 0.00% | 0.333280 | 0 |
| Fully covered, two outputs | 0.00% | 0.00% | 0.333285 | 0 |

No renderer exists after disable; settled paused extension sources are zero.
Blocking-poll/syscall completions are wakeup **proxies**, not total scheduler
wakeups or energy. Software decoding/SHM CPU remains high; no production power
or hardware decode budget passes on this evidence.

The historical loss at cycle 19 remains unexplained. The old interrupted report
has no captured Shell signal/core. Private PID 3194779 and coredump-PID journal
queries for October 30, 19:47–19:51 UTC have no entries. No attribution to the
cache defect is justified; if it recurs, a private GDB backtrace/core and
supervisor exit record are required. All new owned children were reaped and
private session/media removed; no unexpected Shell loss was observed.

## Product gate and honest remaining work

Stability and unlock pass **for the spike**, so they no longer justify a blanket
lifecycle STOP. This implementation nevertheless does **not complete requested
item 3**. The fixture protocol/argv are not design §4. Shipping or installing it
would bypass missing product owner-authenticated desired-state IPC, fixed spawn
policy and sandbox/native qualification. This is an implementation gap, not a
claim that a production extension cannot be written or a newly passed gate.

Still required: independently maintained MIT OR Apache-2.0 GNOME-50 product
source and metadata; exact bounded §4 protocol and negative tests (serials,
scales/enums, no properties, authorization/rate/coalescing/window bounds);
daemon cold-read/events and fixed argv; doctor-visible restart limit plus
explicit Retry; refusing unqualified Shell versions without changing GNOME's
validation; opt-in install/uninstall/status with private-HOME tests and
zero leftovers. Nothing is installed/enabled by default. Re-run the same
stress/security/idle/nine-layout matrix against that product component, not a
renamed Eval-powered spike. Existing importer dispatch must remain intact.

Host GStreamer remains explicitly unqualified; the packaged native sandbox,
decoder allowlist/capability/power gates, physical scanout/sync and native
Windows/macOS execution remain open. Cross-compilation is not native acceptance.
No production CLI or doctor result is represented by an Eval-only spike hook.

## Validation and estimate

Feature-enabled clean-environment Linux workspace: **1080 passed, 0 failed,
14 ignored**. Default and feature-enabled Linux workspace/all-target clippy
and Windows MSVC / Apple ARM all-target checks including all wallpaper crates
pass `-D warnings` with the documented package selection and `blake3/pure`.
Six Python warning/trace gate tests and shell syntax pass. Every cargo command
uses `-j 2`; no cargo fmt ran. Logs and exact commands are in the evidence index.

Run `bash docs/spikes/gnome/qualify-m1a4.sh` only with isolated lab prerequisites.
Large generated files live under ignored worktree `target`, never `/tmp`.
Each supervisor reaps children and removes its private session/media; final
evidence is archived before cleaning generated build/trace files.

Retain **16–26 engineer-weeks remaining for M1**, **74–118 overall**, including
the existing 8–14 integration/security reserve. Do not mechanically subtract
weeks for the cache workaround: production/sandbox/native work is undelivered,
the old Shell loss is still unattributed, and software playback CPU is high.
Confidence improves on the narrow export/re-enable risk, not the full release.
Re-estimate after the product bridge and packaged sandbox qualify.

---

# Historical M1a-3 STOP note (superseded by M1a-4 above)

# 6D-M1a-3: bridge security/idle resolutions; lifecycle still STOPPED

Date: October 30–31, 2026 (Asia/Amman). Branch:
`t3/wallpaper-lifecycle-gates`, reset as requested to local `wave5` at
`b6d14c8`, then implementation commits. **Diagnostic work, not M1 delivery.**
Only the private headless/no-Xwayland Mutter harness was used. Owner desktop,
running one-mk and user extension installation remain untouched.

## M1a-3 decision

**STOP remains.** The independent review's three GNOME findings are fixed in
the spike, and settled renderer polling is removed. The final-code full matrix
fails at cycle 75 with a reproducible bridge `UnknownMethod` while Shell stays
alive. Five final-code clean 100-cycle runs are not demonstrated. The original
private Shell disappearance is not root-caused, and the production-shaped
extension/bridge/CLI/doctor component (task 6) is **not delivered**. Do not
promote the spike or describe these findings as production acceptance.

Primary evidence and attempt classification:
[M1a-3 index](gnome/m1a3-evidence/README.md),
[final security checks](gnome/m1a3-evidence/security-verified.json),
[settled idle measurement](gnome/m1a3-evidence/idle-final.json),
[final lifecycle failure](gnome/m1a3-evidence/qualified-1.json),
[full failing Shell/GDB log](gnome/m1a3-evidence/qualified-1-shell.log).
The unchanged warning baseline remains
[M1a-2 baseline](gnome/m1a2-evidence/baseline-shell.log).

| Gate / measurement | M1a-3 result | Disposition |
|---|---|---|
| PAUSED polling | 3.000 s; renderer/Shell 0 CPU ticks; zero timer returns; 0.333293 blocking-poll completions/s | Own periodic work removed; compositor traffic still wakes renderer |
| Fully covered polling | 3.000 s; renderer/Shell 0 CPU ticks; zero timer returns; 0.666572 blocking-poll completions/s | Same scope; not a literal zero-total-wakeup claim |
| Extension settled overview sources | watchdog/restart/coverage IDs all 0 | Event-driven; no extension periodic timer while paused |
| OFF | renderer absent after disable, owned children reaped | No renderer process left to poll |
| Name squatting / malformed messages | Foreign owner refused before Hello; exact variant signatures, ≤4096-byte bodies, ≤16 monitors, checked canvas; bounded 32-event inbox | Review C(a) fixed in spike; not the complete product protocol |
| FrameReady file ingestion | Method, image mode and Cogl upload path removed | Review C(b) fixed; no compositor-thread frame-file read |
| Replaced renderer window | Re-hide remains until destruction; ≤2 s retirement; retained client killed/reaped | Review C(c) fixed; explicit negative fixture passes |
| Stability, final code | 74 clean cycles; cycle 75 fails Hello with UnknownMethod; Shell alive, OOM counters 0 | **FAIL**; no five final-code clean repeats |
| Pre-security diagnostic repeats | Five consecutive clean 100-cycle runs | Not final-code proof; retained separately |
| Hot-plug notification → first new-window damage | Unplug 37.069 ms; replug 30.588 ms | Compositor-side microsecond timestamps, not physical scanout |
| Unlock → first damage | 4092.193 ms | Explained by shared restart budget's 4000 ms third backoff; not 4.5 s decoder preroll |
| Injected hang | Stale fallback observed 2574 ms; fresh automatic replacement | Partial lifecycle result before repeat failure |
| kill -9 / DING / layouts | Fresh recovery; DING enabled; 1/2/3 outputs at scales 1/2, plus 3 at 1.25, correct extents | Partial final matrix, not a passed full lifecycle gate |
| New warnings versus committed baseline | 0 in final security, idle and failed lifecycle runs | Strict baseline gate unchanged; no allowlist added |
| Production bridge / doctor / extension CLI | Not implemented | **OPEN / undelivered** |

## What changed, and what must happen next

The paused presenter now blocks on persistent private Wayland/GLib FD sources
with no timeout; a per-wait source-add/remove attempt self-woke and was rejected.
The full renderer trace includes worker threads: settled windows show no
5 ms sleeps, timeout polls or timed-out futex waits. Blocking-poll completions
are a strace proxy, not perf scheduler counts. The extension's source IDs are
also checked; unrelated Shell event sources are not attributed to the extension.

The renderer resolves the bridge unique owner through the bus, verifies its
PID/UID against the inherited authenticated Wayland socket peer, then sends
Hello and subscribes only to that unique name. It checks signatures and sizes
before reading children and checks geometry before allocation. The extension
refuses to spawn without owning its name, publishes a latched
`bridge-name-unavailable` diagnostic and log message, pins the renderer unique
sender after exact spawned-PID authentication, and unicasts signals. The
negative tests also bypass the extension guard deliberately to prove the
renderer independently refuses the squatter. These are spike security fixes,
not a claim that GIO cannot allocate transport buffers for arbitrary bus traffic.

The repeated failure is now better isolated: after 74 correct lifecycle cycles,
three successive renderers get `org.freedesktop.DBus.Error.UnknownMethod` for
Hello from the authenticated private Shell connection. The bridge object still
exists in JS and owns its name; no new Shell warning, Shell exit or core is
observed. **Why its exported method becomes unavailable remains unknown.**
Next work needs GJS/GDBus export-registration/lifetime tracing at enable/disable
and GC boundaries, plus retained interface introspection at failure. The older
cycle-19 Shell disconnect still needs independent reproduction with exit/core
evidence; it cannot be attributed to this failure or to OOM from existing data.
Increasing Hello deadlines/retries or hiding warnings would not resolve it.

Compositor-side timestamps replace screenshot-over-IPC gap sampling. They show
damage readiness, not guaranteed painting/physical scanout. Unlock consumes
the same 1/2/4-second budget after the injected hang and kill; the log records
4000 ms scheduled backoff, then approximately 51 ms spawn-to-first-damage.
Keeping the budget matches §4's anti-crash-loop policy; shortening decoder
preroll would not remove this delay. A different recovery policy would require
a deliberate design decision, not resetting the budget on unlock.

The next product increment must still supply independently authored extension
sources and the full §4 v1 messages/rate limits/status, qualified-version
refusal, a separate optional daemon-side component, doctor exhaustion reporting,
explicit user-run install/uninstall/status, private-HOME installation tests and
uninstall cleanliness. Nothing is installed/enabled by default. No shortcut
that disables GNOME validation, uses the owner session or links this into the
daemon hot path is authorized by these diagnostics.

## Validation and estimate

Clean-environment Linux workspace tests with the risk feature enabled:
**1052 passed, 0 failed, 13 ignored**. Linux workspace all-target clippy, both
default and risk-feature-enabled, passes `-D warnings`. Windows MSVC and Apple
ARM all-target clippy passes for input/proto/ctl/app/tray and all four wallpaper
crates with the documented `blake3/pure` cross setting. These are compile-only,
not native-host qualification. Six Python gate tests and shell syntax pass.
Every cargo command uses `-j 2`; no cargo fmt. Evidence includes the initial
wrong-package cross-check's missing lib.exe failure and corrected checks.

Reproduce with `bash docs/spikes/gnome/qualify-m1a3.sh`; generated media/traces
stay under ignored `target/wall-m1-lab`. Each supervisor reaps its own children
and removes the private session. The evidence index distinguishes rejected
iterations, pre-security passes and the final failure; no failed run is counted
as acceptance.

Retain the provisional **16–26 engineer-weeks remaining for M1**, and
**74–118 remaining engineer-weeks overall**, with low confidence until the
export failure and original Shell loss are root-caused. These fixes do not
justify reducing the estimate or marking task 6 delivered. A new estimate is
required after stable final-code repeats and product-component qualification.

---

# Historical M1a-2 resolution (superseded by M1a-3 above)

# 6D-M1a-2: lifecycle resolutions; production remains STOPPED

Date: 2026-10-30. Branch: `t3/wallpaper-m1-lifecycle-gates`.
Starting point: `04e1050` on local `wave5`, including M1a merge `a8053d5`.
Owner authorized all milestones. **Diagnostic work, not M1 delivery.** Nothing
is installed or enabled by default; no owner desktop or running one-mk changed.

## M1a-2 decision and evidence

The owned-xdg/appsink path avoids the original configure warning. A real
extension-disabled baseline, socket-lifetime fix, faster watchdog and bounded
automatic recovery resolve several M1a findings. **Paused renderer polling,
an intermittent private Shell disconnect, capture resolution and the production
protocol remain STOP gates. Task 6 is not delivered.** Do not promote the spike.

Evidence: [disabled baseline](gnome/m1a2-evidence/baseline.json),
[complete baseline log](gnome/m1a2-evidence/baseline-shell.log),
[candidate](gnome/m1a2-evidence/candidate.json),
[complete candidate log](gnome/m1a2-evidence/candidate-shell.log),
[original failure reproduced](gnome/m1a2-evidence/reproduced.json),
[timers fixed but leak persists](gnome/m1a2-evidence/fixed-attempt1.json),
[launcher fixed](gnome/m1a2-evidence/fixed-attempt2.json), and
[interrupted repeat](gnome/m1a2-evidence/interrupted-candidate.json).
The earlier [concurrent-capture run](gnome/m1a2-evidence/concurrent-run/candidate.json)
is retained separately from the corrected final candidate.
Reports retain original PNG hashes/dimensions, sampled pixels and actual capture
request/completion times; retained images are downscaled lossless WebP.
Raw logs preserve original whitespace for digest comparison; the evidence-local
Git attributes disable whitespace lint only for those captured logs.

## Baseline-relative warning triage

Both sessions use the same lifecycle script, private session/system buses,
HOME/XDG and headless Wayland, without Xwayland. The disabled baseline still
exercises DING, layouts, coverage, overview, lock/unlock and 100 extension
requests. Renderer-death operations are inapplicable and explicitly skipped.
All 31 warning/critical records from the earlier successful candidate match the
recorded disabled baseline. The corrected repeat has 32 records (a repeated
baseline signature), still zero unmatched records; occurrence counts are not
deduplicated in the report. Comparison normalizes only logger PIDs, timestamps
and unique D-Bus connection numbers, and includes indented stack traces.
There is **no handwritten allowlist**. Candidate provenance records the baseline
log SHA-256. A matching generic Gjs header cannot hide a new extension stack.

| Baseline record numbers | Count | Signature / attribution | Disposition |
|---|---:|---|---|
| 1–2 | 2 | Unknown framebuffer/Xwayland scaling settings | Reproduced without wallpaper |
| 3 | 1 | Missing colord | Private service absent; baseline reproduced |
| 4–8 | 5 | Missing login1, null user/session proxies, state machine, login-manager promise rejection | Same full stack without wallpaper |
| 9 | 1 | Document portal FUSE mount unknown | Private portal failure; baseline reproduced |
| 10 | 1 | AccountsService user has no username | Baseline reproduced |
| 11–13 | 3 | RealtimeKit properties unavailable | Private service absent; baseline reproduced |
| 14 | 1 | PipeWire unavailable | Private service absent; baseline reproduced |
| 15, 19 | 2 | `meta_window_set_stack_position_no_sync` assertion | Same lifecycle without wallpaper; not attributed to our stacking |
| 16–18, 20–23, 28–30 | 10 | Virtual MetaVendor monitor has no current configuration | Baseline reproduced |
| 24–27 | 4 | Clutter actor needs allocation | Baseline reproduced |
| 31 | 1 | AccountsService password-mode getter type fallback | Baseline reproduced |
| **Total** | **31** | **Zero unmatched records in successful candidate** | **Narrow no-new-warnings gate passes** |

Reproduction establishes non-attribution for these observations, not that Mutter
assertions are harmless on physical machines. DING's ordinary trash/metadata
failure messages remain in complete logs; they are not warning exclusions.

## Root cause and fixes

**Retained windows belonged to exited renderer PIDs, not leaked extension
actors.** Census now records stable window IDs, PID, actor presence and hidden
state. `Gio.SubprocessLauncher` retained its copy of the Wayland socket until JS
garbage collection. Reaping the child alone did not disconnect Mutter's client.
Closing the launcher in `finally` immediately after
`Meta.WaylandClient.new_subprocess` releases that descriptor. Successful runs
have exactly one managed window after enable and **zero after every disable**,
including replacement windows on layout changes. Final census has only DING's
two desktop windows and no extension leaks. No ghosts are hidden/deleted to fake
the census. Unnecessary raised/lower and deferred pre-map stacking are removed.

The watchdog uses a 2.5 s stale threshold and 100 ms sampling only while an
adopted renderer is unpaused. Pause/teardown cancels it. Hang falls back to native
still wallpaper and terminates the child instead of unbounded repaint probes.

Automatic recovery shares child ownership and budget across disable/re-enable:
1/2/4 s backoff, at most three starts per rolling 60 s, then exhausted latched
until explicit Retry. Children cannot overlap; waits call `wait_finish`; stale
bridges cannot restart; pause cancels pending restart. Death and lock/unlock
tests do not manually start a replacement. Exhaustion exposes
`failed-retry-limit: static wallpaper; explicit Retry required` in spike events,
with PID null and timer zero after re-enable. This is **Eval-visible laboratory
state, not product doctor integration**.

## Measured lifecycle results

The concurrent-capture successful run below was recorded at
2026-10-30T19:36:10Z. JSON contains each run's exact provenance and timings.
The corrected final repeat at 19:51:31Z also completed 100 cycles with 1/0
windows, zero leaked actors/clones, and no narrow gate failures: startup
183.478 ms, hung fallback **2633.25 ms**, cycle damage min/median/max
154.459/173.048/206.215 ms, and automatic restart waits
1309.04/2270.99/4218.78 ms before the fourth-death exhausted latch.
Kill-to-first-damage was about 2086 ms and unlock-to-first-damage about
4503 ms from Shell monotonic events. Its final still sample was requested at
4422.50 ms/completed at 4696.27 ms, followed by video at 4778.10–4879.49 ms.
Paused child CPU was 1.00% of one core. Maximum hot-plug request intervals
were 224.34/349.96 ms; it still does not establish fine visible-gap bounds.
This repeat does not erase the intervening cycle-19 disconnect.

| Case | Observation | Result / limitation |
|---|---|---|
| Baseline-relative log | 31 matching records, zero new | Narrow pass |
| Startup first video damage | 170.514 ms | Trusted generated VP9 only |
| SIGSTOP fallback | 2563.68 ms; earlier launcher-fixed run 2600.62 ms; interrupted repeat 2628.41 ms | Under 3 s, observed margin about 370–436 ms |
| 100 cycles | Every successful cycle 1 enabled / 0 disabled; damage min/median/max 151.805/154.954/167.613 ms | Successful runs, not a stability guarantee |
| Live SIGKILL | About 2073 ms kill-to-first-damage from Shell events | Automatic 2 s backoff in this cascade |
| Unlock | About 4486 ms action-to-first-damage; still capture 4455.86–4701.14 ms, next video 4774.98–4872.40 ms | Automatic; multi-second visible still gap |
| Restart cap | Three fresh replacements; fourth death exhausted, PID null, timer zero; persists after re-enable | All four entries in `fixed-attempt2.json` |
| Unplug earlier run | Still sampled 108.92–360.30 ms after action; video 455.34–527.30 ms | Gap sampled; duration only coarsely bounded |
| Replug earlier run | Still sampled 91.92–354.06 ms; video 432.13–534.43 ms | Gap sampled; duration only coarsely bounded |
| Concurrent hot-plug | No still sampled; maximum request intervals 220.49/218.18 ms unplug/replug | **Not zero gap**; 50 ms cadence not achieved |
| Paused overview | Child 1.33% of one core over 3 s; extension watchdog/restart IDs zero | **FAIL:** animation child remains alive, polling at 5 ms |
| Virtual layouts | 1/2/3 monitors, scales 1/2 and 3 at 1.25 match extents/clones | Not physical GPU/HiDPI qualification |
| Repeated qualification | Shell disconnected during cycle 19 after 18 completions; D-Bus NoReply | **Unresolved stability failure**, cause not established |

Capture intervals above are request/completion windows, not exact exposure
times or gap durations. A 50 ms schedule does not achieve 50 ms actual IPC
sampling. Unlock's black frame may be the lock screen, not live video. Need
compositor/frame-clock capture or bounded private screencast before claiming
fine hot-plug bounds. Failed runs remain evidence even if a later repeat passes.

## Remaining STOP gates and re-estimate

Task 6 remains unimplemented: generic Eval argv hooks and the spike broadcast
interface are not the design's bounded product protocol. The test wrapper
refuses anything except Shell 50.1; that is **not** production version gating.
No installer/uninstaller is added, and the owner's extension directory is untouched.

1. Diagnose the private Shell disconnect using supervisor exit/core evidence;
   repeat lifecycle tests without losing the compositor.
2. Replace paused renderer polling with event-driven shutdown/recreation; prove
   no animation child or periodic work while paused/off, including lock/coverage.
3. Implement `org.onemk.WallpaperBridge1`, fixed argv/daemon authorization,
   unique-name plus exact PID/UID binding, bounded Hello/state/layout messages,
   deadlines/rates, unicast signals, owner-loss handling and negative tests.
4. Add visible/doctor-reportable exhausted state, explicit Retry, qualified
   Shell refusal and `one-mk wallpaper`-only install/uninstall; preserve
   MIT OR Apache-2.0 and default OFF.
5. Resolve actual capture cadence, then qualify hostile-media isolation, native
   hosts, physical power/HiDPI, GPU copies, sync, sleep and soak gates.

Allowance **retained**, not reduced by diagnostic progress: M1 16–26
engineer-weeks; M2 10–16, M3 16–24, M4 10–16, M5 14–22, reserve 8–14;
**74–118 remaining engineer-weeks total**, not calendar dates. No M1 completion
or production-shaped extension delivery is claimed.

## M1a-2 reproduction and validation

`bash docs/spikes/gnome/qualify-m1.sh` builds offline with `-j 2`, records a real
baseline then candidate under ignored `target/wall-m1-lab`, and refuses Shell
versions other than 50.1. Its isolated supervisor uses private buses/HOME/XDG
and reaps owned children, generated media and session files. Private Unix-socket
approval may be needed. Missing lab dependencies never authorize the live desktop.

Clean-environment validation: feature-enabled `one-mk-wall` 2 unit + 3 binary
tests; default wall 3 binary tests; wall-sync 12 integration tests; four Python
gate tests; shell syntax; Linux workspace all-target clippy default and enabled;
Windows MSVC/Apple ARM all-target cross-clippy including wallpaper crates,
all with `-D warnings`. [Logs](gnome/m1a2-evidence/validation.txt) and
[default checks](gnome/m1a2-evidence/validation-default.txt). No formatter run.
Cross checks do not qualify native runtime hosts. Matrix evidence is explicitly
partial risk evidence, **not green M1 acceptance**.

---

# Historical M1a resolution (superseded by M1a-2 above)

The following is retained verbatim as the prior investigation and estimate;
its strict-warning rule and open defects describe that earlier run, not the
current baseline-relative gate.

# 6D-M1a: configure warning resolved in a diagnostic; lifecycle still STOPPED

Date: 2026-10-30. Branch: `t3/wallpaper-mutter-presentation`.
Baseline: `76534af` (`Merge wallpaper M1 probe and stop evidence into wave5`).
Owner authorized all milestones. **Neither M1a qualification nor M1 delivery
is complete. Do not deploy this binary into the owner's desktop.**

## Decision and primary evidence

Retain the own-xdg-surface/appsink path as the next presentation experiment,
not a shipping renderer. It removes the original initial-content warning,
but the strict full-session gate still fails: Mutter stack/allocation warnings,
retained managed windows, a late watchdog, and no bounded restart cascade.
Do not suppress these warnings or mark qualification complete. No decoder
sandbox, untrusted importer, production bridge or new receiver authority is added.

The [original M1 report](gnome/m1-presentation-original.json) is unchanged.
New evidence: [final matrix](gnome/m1a-evidence/matrix.json),
[complete final log](gnome/m1a-evidence/shell.txt),
[comparison runs](gnome/m1a-evidence/comparison.json),
[wire trace](gnome/m1a-evidence/configure-trace.txt),
[45-capture contact sheet](gnome/m1a-evidence/contact-sheet.webp).
Individual captures are downscaled lossless WebP; JSON retains original PNG
dimensions, SHA-256, scheduled/request/completion times and sampled pixels.
Generated full-size PNGs, fixture, plugins and session files are removed.

## Root cause and candidate paths

Host GNOME/Mutter 50.1, GStreamer 1.28.2. The extension's DESKTOP window and a
NORMAL-window control both reproduce the warning. DESKTOP conversion is
**not necessary** for this initial-content warning.

The wire trace shows `xdg_surface.configure(1)`, `ack_configure(1)`, then
multiple **empty root commits**, followed by the first video-subsurface and
root-border buffer commits. There really is an ack on the wire; saying
"waylandsink never acknowledges configure" would be incorrect.

Matching upstream sources inspected during diagnosis (not vendored):

- GStreamer tag `1.28.2`, `gst-libs/gst/wayland/gstwlwindow.c`,
  `handle_xdg_surface_configure` (line 185): acknowledges, marks configured,
  calls `gst_wl_window_update_geometry` (line 1006), which commits root state
  before the first root buffer is attached.
- Mutter tag `50.1`, `src/wayland/meta-wayland-xdg-shell.c`,
  `meta_wayland_xdg_toplevel_post_apply_state` (around line 1036): window
  finish-move/resize processing needs a surface buffer.
- Mutter tag `50.1`, `src/wayland/meta-window-wayland.c`,
  `acquire_acked_configuration` (around line 1360): first nonempty state has
  no pending ack, pending configurations exist, and no last-acked configuration
  exists, so it warns and works around the missing state.
- Stable `xdg-shell.xml`: initial empty commit precedes configure; acknowledge
  before content commit; never acknowledge a serial twice. This identifies
  the installed Mutter/GStreamer state interaction, not a conclusive protocol
  conformance verdict about every compositor.

| Candidate | Actual experiment | Decision |
|---|---|---|
| `waylandsink` | Warning reproduced with DESKTOP and NORMAL controls | Reject standalone path on measured stack |
| `gtk4paintablesink` in GtkWindow | [No such plugin](gnome/m1a-evidence/gtk4-availability.txt) in this lab; no GtkWindow playback test possible | **Not runtime-tested**; provision privately for subsequent comparison |
| Rust `wayland-client` xdg root + GStreamer appsink | Real renderer, one VP9 decoder; initial empty commit → configure → ack immediately with first nonempty root commit | Original warning absent; retain diagnostic only; complete log still fails |

The extension owns the connection through `Meta.WaylandClient`, identifies
windows using `owns_window`, sets DESKTOP before initial map/configure, hides
the source actor, and clones above native wallpaper/below DING. NORMAL windows
are not a shipping fallback: work-area constraints break the spanning extent.
The Rust prototype uses an empty input region, two release-tracked memfd
buffers, current-surface configure serials and frame callbacks. Layout changes
destroy the old role before replacement; this is not seamless hot-plug.

New dependencies `wayland-client 0.31.15` and `wayland-protocols 0.32.13`
declare MIT. GStreamer/appsink and GLib/GIO remain dynamically linked LGPL
laboratory dependencies; no GPL upstream implementation is copied into this
repo. Selected plugins and forbidden-library string checks are still **not**
a release SBOM/licence audit. Fixed 3840×1080 BGRx fixture, CPU nearest-neighbor
scaling, copies and dynamic FFI are unqualified: no GPU zero-copy, physical
HiDPI, hostile-media security or production unicast/PID-binding claim follows.
The feature remains default-off and Linux-only.

## Final measured lifecycle

Run began `2026-10-30T18:47:01.029908+00:00`, optimized real Rust binary,
three headless virtual 1920×1080 outputs. First-frame figures mean **supervisor
observation of Shell damage**, including IPC/polling, not panel scanout.
CPU is process tick delta over three seconds, percent of **one core**; short
windows/host contention limit interpretation. Shell PID is found only among
this run's private leader descendants, not by a reused global runtime path.

| Case | Measurement | Qualification |
|---|---|---|
| Startup, three outputs | First damage **159.947 ms**; 5760×1080 buffer, three clones; DING rather than source has focus | Narrow startup pass |
| Scale 1, 1/2/3 outputs | 1920/3840/5760×1080 buffers, 1/2/3 clones | Logical extent observed |
| Scale 2, 1/2/3 outputs | 960/1920/2880×540 buffers, 1/2/3 clones | Not physical sharpness qualification |
| Fractional scale 1.25, three outputs | 4608×864 buffer, three clones | Rotation/mixed scales not tested |
| Unplug | Video in first capture, request 82.41 ms / completion 176.80 ms | No static gap sampled; not proof of zero gap |
| Replug | Static in first capture (89.48–347.95 ms request/completion); video next (425.32–530.64 ms) | Visible still gap; exact duration not recoverable from IPC captures |
| Overview pause, two outputs | Renderer **1.00%**, Shell **1.33%** | Actual PAUSED pipeline; residual polling is not zero-work |
| All-covered, two fullscreen windows | `paused:true`, `reason:covered`; renderer **0.67%**, Shell **0.67%** | Coverage actually triggered |
| Playing, three outputs | Renderer **60.99%**, Shell **9.67%** | Software copies/scale; not an efficiency pass |
| Hang (`SIGSTOP`) | Static fallback observed **3732.27 ms**; Shell event stale 3617 ms | **Fails 3 s**; one-second watchdog sampling overshoots |
| Live `SIGKILL` | Fresh damage/two clones verified after SIGCONT, then killed; static first capture completed 255.55 ms through last at 2925.54 ms | Fallback observed; manual next start **185.579 ms** to damage, not automatic recovery |
| Lock/unlock | `locked:true`, mode `unlock-dialog`; clones removed; lock frames through 2093.18 ms; static desktop unlock captures completed 370.32–2926.48 ms | Manual restart after burst **170.501 ms**; not an automatic-unlock gap measurement |
| Restart cascade/cap | 6B has no automatic bounded restart implementation | **Not qualified**; manual starts are not a cap test |
| Disable/enable ×100 | 100 old renderer PIDs exited; damage min/median/max **154.560 / 164.570 / 279.350 ms** | Managed wallpaper-window count **1–24**, not invariant one |
| Final leak census | No named one-mk actors/stage clones, bridge absent; **four managed wallpaper windows remain after one second** | **Fails window-leak gate**, not dismissed as animation |
| DING | Enabled state 1; `Desktop Icons 1/2` in final managed-window list | Runs; private missing GVfs trash/metadata support still logged |
| Complete Shell log including teardown | **31** WARNING/CRITICAL/Gjs-error matching lines; **zero** original initial-content warnings | **Fails strict gate**, no allowlist |
| Process/session cleanup | Children reaped; private media/files/socket removed; host-namespace exact-runtime scan found **zero processes** | No remaining private processes; does not erase retained-window failure |

Screenshot targets are 0/250/…/2000 ms. Screenshot/snapshot IPC sometimes
exceeds 250 ms: kill/unlock bursts finish about 2.9 s. JSON retains actual
request/completion times, not fictitious exact intervals. Pixel (500,700)
distinguishes this fixture's black video background from blue wallpaper;
black lock-screen pixels are **not** called video. These are coarse visible
observations, not presentation-time feedback or precise gap bounds.

Earlier CPU values sampled a stale private Shell PID and are excluded. Earlier
kill-9 captures taken while already stale are not the live test above. Early
baseline cleanup permission failures left private processes; they were later
matched by UID/exact private runtime and terminated. No live owner Shell or
one-mk was touched. Earlier DING failures coincided with UID inotify exhaustion;
DING runs in the final session after cleanup, without claiming all GVfs works.

## Strict harness and reproduction

`m1-risk-probe.py` fails on **all** WARNING/CRITICAL/Gjs-error/JS-error lines,
including startup services and teardown; `test_m1_gate.py` tests classification
without an allowlist. Matrix invariants independently fail late watchdog,
retained windows and absent restart cap. Final exit **1**, explicit failures
in JSON. GTK comparison is an explicitly missing test, never a mock pass.

```sh
env -i HOME=/home/samer PATH=/home/samer/.cargo/bin:/usr/bin:/bin \
  cargo build --offline -j 2 --profile bench-opt -p one-mk-wall \
  --features gnome-risk-probe
env -i HOME=/home/samer PATH=/usr/bin:/bin PYTHONDONTWRITEBYTECODE=1 \
  python3 docs/spikes/gnome/m1-risk-probe.py --matrix --sink own \
  --binary target/bench-opt/one-mk-wall --label matrix-qualified-attempt
# Original sink comparison; add --normal-window for the type control:
env -i HOME=/home/samer PATH=/usr/bin:/bin PYTHONDONTWRITEBYTECODE=1 \
  python3 docs/spikes/gnome/m1-risk-probe.py --sink wayland --trace \
  --label desktop-trace --binary target/bench-opt/one-mk-wall
```

Only the existing isolated harness is used: clean environments, private session
and system buses, HOME/XDG, memory settings, headless/no-X11 Mutter and private
extension copies. Restricted CLI environments may need private-socket approval.
Never target the live session or owner's extension directory. Generated fixture
hash/private-peer guards are laboratory guardrails, **not** a hostile-media sandbox.

## Next-best design and re-estimate

**Stop qualification here: no warning-free complete path demonstrated.** This
does not prove such a path impossible. Next-best design is the owned xdg root
with acknowledged first buffer, still one decoded scene/N Shell clones, with
native static wallpaper as default/failure state. Before product work resumes:

1. Diagnose retained managed windows and `meta_window_set_stack_position_no_sync`
   assertion with Meta.WaylandClient/window/actor lifetimes; add minimal own-root
   close/destroy control, distinguish Shell stack bugs from extension operations.
   Do not hide/delete ghosts to fake a passing census.
2. Privately provision `gtk4paintablesink`; compare real GtkWindow with the same
   connection-owned DESKTOP adoption, licence and one-decoder checks. Toolkit
   lifecycle management remains a candidate, not evidence.
3. Give the private lab required services/settings **without** attaching to the
   live system bus; resolve rather than filter AccountsService/logind,
   experimental-setting, portal and Clutter warnings. Fake GDM alone is not a
   representative lock/session environment.
4. Implement/test the design's bounded restart budget and 3 s damage deadline;
   rerun lock/unlock cascades, live crash, captures, leak census and 100 cycles.
   Investigate residual paused polling, then GPU copies/physical HiDPI/power.
5. Only after warning-free lifecycle qualify hostile-media isolation, native
   hosts, LGPL replacement/artifacts and all physical design §11 acceptance.

Planning allowance: **M1 remaining 16–26 engineer-weeks**, versus 14–22 after
the original stop. M2 10–16, M3 16–24, M4 10–16, M5 14–22, reserve 8–14:
**74–118 remaining engineer-weeks total**, not elapsed calendar dates. Extra
2–4 weeks cover lifecycle/toolkit/lab uncertainty, not a promise of a small fix.
Frozen receiver authority, default-off and physical acceptance remain unchanged.

## Validation

Clean environments, offline Cargo with `-j 2`; no formatter. [Transcript](gnome/m1a-evidence/validation.txt).

| Check | Result |
|---|---|
| Feature-enabled `one-mk-wall` tests | Pass: 2 unit + 3 real-binary tests |
| Default `one-mk-wall` / `one-mk-wall-sync` | Pass: 3 binary + 12 sync integration tests |
| Workspace clippy, all targets, `-D warnings` | Pass |
| Feature-enabled wallpaper clippy, all targets, `-D warnings` | Pass |
| Windows MSVC / Apple ARM cross clippy | Pass, including wallpaper crates and enabled risk feature; native hosts untested |
| Python strict-log gate tests | Pass: 2 tests |
| Real final private Mutter matrix | **FAIL**, as detailed above; not green acceptance |

Cross targets: `x86_64-pc-windows-msvc`, `aarch64-apple-darwin`; packages
`one-mk-input`, `one-mk-proto`, `one-mk-ctl`, `one-mk-app`, `one-mk-tray`,
`one-mk-wall`, `one-mk-wall-sync`. Earlier full-workspace runtime tests had
unrelated daemon/environment failures (see prior revision); not rerun or
claimed green here. No physical multi-host sync, sleep, 24 h soak or independent
security/legal review is supplied by this work.
