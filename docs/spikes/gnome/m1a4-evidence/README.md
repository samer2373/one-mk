# M1a-4: bridge lifetime and unlock recovery

Captured October 30–31, 2026, on `t3/stabilize-wallpaper-bridge`, starting at
`wave5` (`7684a99`). **Headless diagnostic qualification, not M1 delivery.**
All sessions used the repository's isolated Mutter supervisor: private session
and system buses, HOME/XDG, generated trusted media, no Xwayland. Nothing was
installed into the owner's extension directory, enabled on the owner's desktop,
or connected to the running production daemon. No GNOME version-validation
setting was changed. Host GStreamer 1.28.2 remains unqualified for production.

## Dispatch failure: reproduction and diagnosis

`original-gc.json` reproduces the actual `UnknownMethod` after 21 completed
cycles: the renderer repeatedly receives `No such method “Hello”`, while the
private Shell stays alive. `original-cache.json` catches the inconsistent
interface before spawning at cycle 79: `info.methods` contains `Hello`, the
exported introspection XML lists it, but `info.lookup_method("Hello")` is null.
Both runs force GJS collection between generations; both have zero unmatched
baseline warnings. Their complete Shell/GDB logs are retained, not just tails.

The source-level cause is the lifetime of the introspection lookup cache, not
a renderer calling the wrong object or a missing JS `HelloAsync` handler:

- Installed GJS 1.88.0 `Gio.js`, `_wrapJSObject`, line 444 builds the interface
  lookup cache. It does not balance that additional build with a release.
- GJS 1.88.0 `gjs_dbus_implementation_finalize`, line 194, unreferences the
  interface. The wrapper's additional cache reference is not released there.
- GLib 2.88.0 `g_dbus_interface_info_cache_build` indexes its process-global
  cache by the non-owning native interface pointer; method-name keys and method
  pointers also borrow the interface's storage. Its lookup takes the cached
  result without falling back to the current method array. Interface unref
  frees that storage without removing an independently retained cache entry.
- Thus per-enable interface allocation/GC can leave stale cache entries and
  subsequent pointer reuse can select the wrong cache. The inconsistent lookup
  is directly observed; allocator address reuse was not separately traced.
  `gdbusconnection.c` maps the missing lookup to the exact observed error.

Installed package versions: `gjs 1.88.0-1`,
`libglib2.0-0t64 2.88.0-1ubuntu0.1`, GNOME Shell 50.1. `manifest.json` records
the hashes of the extracted installed override and inspected version-tagged
GJS/GLib source. The source tags are `GNOME/gjs` 1.88.0 and `GNOME/glib` 2.88.0;
this is not a claim that every downstream patch was audited. No upstream code
is copied into the extension.

The fix creates a single module-held `Gio.DBusInterfaceInfo`, constructs
`Gio.DBusExportedObject` directly, and explicitly dispatches methods, avoiding
`wrapJSObject`'s extra cache build. Each enable exports exactly once and holds a
strong `_dbus` reference. Disable invalidates the generation, stops/reaps its
child, unexports, then relinquishes the name. Acquisition is non-replacing and
non-queuing; loss refuses playback. A 500 ms self-call to the unique Shell
connection confirms dispatch readiness before any renderer is started. Late
callbacks cannot revive a destroyed generation. Failure latches; it does not
poll forever. The spike-only `Ready` method is **not** added to product v1.

The renderer's Hello has at most ten 100 ms calls with 25 ms between attempts
(less than approximately 1.25 seconds, excluding scheduling). Only transient
dispatch errors are retried; authorization/version/permanent failures stop
immediately. Owner PID/UID pinning is unchanged. The Rust unit test checks the
error classification. This is a fallback, not a substitute for fixing export.

## Final-code stress evidence

Each final stress session starts a fresh private Shell under GDB and forces GC
on each generation. There are five consecutive successful 100-cycle sessions,
then a successful 500-cycle session. Every cycle checks introspection versus
method lookup, real renderer damage, child exit and zero managed windows after
disable. The renderer/extension source and binary are identical across these
six sessions and the matrix; see source and binary hashes in the manifest.

| Report | Completed | Observation timer min / median / max, ms | New warnings |
|---|---:|---:|---:|
| final-100-1 | 100 | 144.706 / 149.256 / 236.833 | 0 |
| final-100-2 | 100 | 162.486 / 184.334 / 344.193 | 0 |
| final-100-3 | 100 | 161.919 / 168.214 / 285.073 | 0 |
| final-100-4 | 100 | 150.836 / 169.757 / 305.075 | 0 |
| final-100-5 | 100 | 147.138 / 186.121 / 726.873 | 0 |
| final-500 | 500 | 144.988 / 150.133 / 826.880 | 0 |

These timers start **after enable and the introspection/GC checks**. They are
not enable-to-frame or unlock-to-frame measurements. Use compositor timestamps
in the matrix for unlock. A further 100 cycles pass in each final full matrix.
The Python collector changed later only to cover all nine scale/output pairs
and collect playing syscall counts; these do not change the extension/renderer.

Retained non-acceptance attempts: `fixed-gc-1` failed after one cycle because
the collector explicitly restarted an already automatically resumed renderer,
then inspected it before reaping. The collector now observes the automatic
resume rather than killing it. `fixed-gc-2` is the following clean 100-cycle
diagnostic, **not** one of the five consecutive final qualification sessions.
Neither original failure nor the collector failure is discarded or counted as
a passing final run.

## Unlock versus crash recovery

Destroying a healthy adopted renderer records one pending lifecycle resume.
Fresh enable must first acquire the name and pass readiness; if unlocked and
otherwise runnable it consumes that token and schedules one immediate idle
restart. Deliberate lifecycle resumes do not spend or clear crash history.
Crashes retain the 1/2/4-second delays and three-start rolling budget. Exhaustion
remains latched through disable/enable, with no restart source. This diagnostic
latch is not yet integrated with product doctor or a product Retry operation.

`final-matrix.json` records locked=false at 810281798937 µs and the first new
renderer damage at 810281892075 µs: **93.138 ms**. From the scripted unlock
request to that damage is **451.024 ms**, including the Shell unlock transition.
The prior M1a-3 notification-to-damage measurement was 4092.193 ms; its shared
4000 ms crash backoff is no longer used for healthy lifecycle resume. These are
compositor observations, not physical scanout or proof of no visible paint gap.
The later nine-layout matrix is retained separately; neither run is replaced.
That repeat measures **109.128 ms** notification-to-damage and **452.663 ms**
request-to-damage, again comfortably below one second.

## Historical Shell disappearance

The M1a-2 interrupted capture completed 18 cycles, then lost the private Shell
PID 3194779. That old capture has no supervisor exit/signal/core evidence.
The retained journal queries, restricted to that PID and coredump PID for
2026-10-30 19:47–19:51 UTC, both return `-- No entries --`. This is **absence
of evidence**, not evidence that no crash happened. Do not attribute that loss
to the cache defect without a signal/core. No unexpected Shell disappearance
occurs in the new final-code stress, matrix, security or idle sessions. If it
recurs, keep the isolated GDB stop/backtrace/core and supervisor report before
cleaning the session. The harness writes these under the ignored worktree lab,
not the small `/tmp` disk, and never changes host core handling.

## Matrix, power, security and warning gate

`final-matrix.json` is the initial seven-layout matrix, with 100 cycles and no
qualification failures. `final-matrix-nine-layouts.json` is the final nine-layout
repeat: 1/2/3 outputs at each of scale 1, 2 and 1.25, including another 100 cycles.
Both retain actual compositor events, damage, extents, child/window census and
the crash-cap checks. The resolution note gives the measured table.

`final-security.json` passes name-squatting refusal, pre-Hello refusal,
FrameReady absence, re-hide of retired windows and kill/reap of an uncooperative
replacement; Shell remains responsive. The deliberate guard-bypass fixture is
Eval-only and confined to the private session. It is not a product API.

Matched CPU/wakeup-proxy windows in `final-idle.json` and the full compressed
renderer strace (three seconds each, CPU percent of one core):

| State | Renderer CPU | Shell CPU | Blocking-poll completions/s | Timer returns/s |
|---|---:|---:|---:|---:|
| Playing, 3 outputs | 59.66% | 9.00% | 29.994510 | 125.643671 |
| Overview paused, 2 outputs | 0.00% | 0.00% | 0.333280 | 0 |
| Fully covered, 2 outputs | 0.00% | 0.00% | 0.333285 | 0 |

Blocking-poll/syscall completions are **proxies**, not scheduler/perf wakeup
counts or whole-machine power measurements. Settled paused source IDs are all
zero; the disabled renderer is absent. Software decode/SHM playing CPU remains
high and does not qualify the proposed product power budget.

Every retained final run has zero unmatched warning records against the exact
committed M1a-2 disabled baseline, hash
`b0b3c293e1d88e8bc6dc1614b022f4302dae678831709d47221552527ddc9c52`.
Comparison still includes stack traces; no allowlist or baseline modification.
Known baseline warnings/critical records remain in complete logs; zero *new*
warnings is not zero total warnings.

## Reproduction and validation

From the repository root, with the existing GNOME 50.1/GJS/GStreamer/GDB/strace
lab prerequisites, run `bash docs/spikes/gnome/qualify-m1a4.sh`. It builds offline
with `cargo ... -j 2`, then runs five 100-cycle GC sessions, 500 cycles, the full
matrix, security negatives and idle trace in sequence. First failure stops it.
All child processes, session directories and generated media are reaped/removed
by each supervisor. Generated results stay in ignored `target/wall-m1-lab`.
Do not run two supervisors concurrently: they intentionally share one lab path.

To reproduce the old export using the new collector, copy `docs/spikes/gnome/ext`
to an ignored worktree directory, replace only its wallpaper `extension.js`
with `git show 7684a99:docs/spikes/gnome/ext/wallpaper-spike@one-mk.test/extension.js`,
then run the probe with `--extension-source` pointing at that entire copied
extension tree, `--stress-only --cycles 100 --gc-stress --debug-shell`, the
committed `--baseline-log`, a built `--binary`, and a new `--label`. The override
is copied into **only the private session**, never the user's extension path.
Failure cycle numbers vary with allocation and GC; they are not a fixed 75.

Validation logs: `tests-clean.log` records **1080 passed, 0 failed, 14 ignored**
for the clean-environment feature-enabled Linux workspace. Four clippy logs
record Linux workspace/all-target default and risk-feature builds plus Windows
MSVC and Apple ARM all-target checks including wall/wall-sync/wall-canvas/
wall-import, all `-D warnings`. Cross-checks use `blake3/pure` and the documented
input/proto/ctl/app/tray selection; they do not link/run the native daemon or
qualify a Windows/macOS host. Six Python gate tests and shell syntax pass.
No cargo fmt ran. The manifest retains the commands and artifact/source hashes.

## Explicitly not delivered

The spike is not the production component. Product §4 uses different exact
signatures, serials/scales/enums, no properties, owner-authenticated daemon
desired state and fixed argv. The current spike still uses Eval-controlled
fixture argv and a reduced laboratory protocol. Production source/metadata,
full bounded product bridge, doctor-visible limit, product Shell-version guard
and `one-mk wallpaper extension install|uninstall|status` are **undelivered** in
this change. The harness's existing 50.1 refusal is not a product version check.
No installer is exposed for an unqualified diagnostic. Importer dispatch in
`main.rs` is unchanged. The full product lifecycle matrix cannot be claimed from
the spike results. See the resolution note for the remaining implementation and
qualification gates and retained estimate.
