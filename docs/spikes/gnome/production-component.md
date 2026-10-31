# GNOME component implementation checkpoint — NOT a playback qualification

## Step C: lifecycle and restoration in the laboratory (2026-10-31)

The extension and laboratory renderer now survive layout change and hot-plug,
lock/unlock, suspend/resume, kills, disable/uninstall and Shell restarts with
real playback, and every teardown leaves the user's own wallpaper on screen;
see the [step C evidence](m1-lifecycle-evidence/README.md). Changes to the
production extension (all under the unchanged default-off authorization):

- Stops are cooperative: SIGTERM, SIGKILL only after 2 s (escalation timers
  live in the recovery object, so they survive disable). The renderer handles
  SIGTERM by removing its worker, cgroup and surface before exiting. This
  closes the earlier deviation "immediate force-exit" (item 4 below).
- A window adopted under an older layout serial is never revealed again, the
  layer is hidden whenever no fresh window is behind it, and an unchanged
  logical layout is not a layout change. (Before: a late damage of the old
  window re-marked it fresh, so frames sliced for the old layout could be
  shown on the new one for up to 100 ms.)
- Suspend: logind's `PrepareForSleep(true)` is pause reason 8 until resume;
  wake, like unlock, starts a fresh renderer at once.
- Bridge state 4 *holding*: a still or a long frame is announced instead of
  being re-committed every 2 s; the Shell runs no damage watchdog meanwhile.
  The first-damage-before-geometry case is re-checked (bounded, 40 x 25 ms)
  because a still sends no second damage.
- Runs the Shell ends for a pause reason, lock, sleep or disable are credited
  back: only failed runs reach the three-per-minute cap (before, four
  lock/unlock or cover/uncover cycles in a minute latched "restart limit
  reached" for the whole session). The first healthy frame cancels the 5 s
  start deadline; a renderer paused by its plan is not failed by it.
- The bridge records the last stop's cause (`lastStop`) for diagnosis.

Renderer: SIGTERM teardown, Holding/Playing reports per playback generation,
no keepalive commits, a spurious `EAGAIN` from the Wayland read is no longer
fatal (it killed a renderer during a layout change), and stale playback groups
of a SIGKILLed predecessor are reclaimed at start. Daemon publisher: a local
layout outside the committed canvas is published paused (static), and the Gate
admits such a paused plan; a playing plan must still lie inside the canvas.

## Step B: real playback in the laboratory (2026-10-31)

With the laboratory features, the renderer now plays the admitted plan's
imported media: a sandboxed playback worker (the importer's bubblewrap/seccomp/
cgroup model) decodes GIF, APNG, animated WebP, still PNG, VP9 and AV1; the
renderer presents one xdg surface whose clones show each monitor's exact slice
of the shared canvas, never the reserved absent-computer slice. Screenshots,
loop seams, pause, 1080p30 resources and 100 real-playback cycles with zero new
warnings are in the [step B evidence](m1-playback-evidence/README.md). One
extension bug surfaced (first damage before buffer geometry stopped every real
renderer) and is fixed. Production authorization stays false; lifecycle (step
C) and production packaging remain open.

## Step A: authenticated daemon presentation session, laboratory (2026-10-31)

The laboratory daemon (`wallpaper-m1-lab`) now authenticates to the renderer the
production extension spawns (`m1-lab`, `--present=gnome --m1-lab`) and publishes
plans the renderer admits through `Gate::accept_encoded`; consent revocation and
daemon exit/SIGKILL tear the presentation down and end the renderer within
milliseconds. 100 real-process start/stop cycles passed with zero new warning
records. The extension and bridge XML are unchanged; the renderer still creates no
surface or decoder, and production authorization stays false. Steps B–D remain
open. See [the session design](presentation-wire.md) and
[evidence and limits](m1-presentation-evidence/README.md).

## Encoded admission precursor follow-up (2026-10-31)

`f93761a` adds a bounded payload codec and encoded Gate entry point, with ten
additional unit tests. This is **not daemon IPC**: no production process calls
the new entry point, no new Shell bridge method is added, and authorization stays
false. Requested stage A is not reached; B/C are not attempted and D is not enabled.
The rerun of the private GNOME 50.1 regression again passes 100 paused-fixture
cycles with zero new baseline warning records, not real playback qualification.
See the [wire boundary](presentation-wire.md) and
[exact evidence/limitations](encoded-admission-evidence/README.md).

## Admission precursor and private child stress follow-up (2026-10-31)

An I/O-free presentation admission state machine now exists, with eleven
failure-path tests for pinned transport identity, consent revision/generation,
sequence replay, imported media hash, frozen reserved canvas, monitor slices,
pause composition, fresh plans after suspend/hot-plug, and terminal revocation
or owner loss. It is **not wired into daemon/renderer IPC** and does not itself
authenticate credentials or control a process. Production authorization remains
false; the normal binary still refuses playback.

The private GNOME 50.1 component runner now proves 100 authenticated **paused
fixture** child start/stop/GC cycles, every exact PID and child handle reaped,
no background URI changes, and zero new warnings against the unchanged M1a-2
baseline. Existing conformance and negative cases also passed. The first
restricted start failed; the approved isolated rerun passed. No live desktop was
used. These are not media/playback/restoration or daemon-crash lifecycle results.
See the [laboratory instructions](README.md), [retained exact evidence and
limitations](component-admission-evidence/README.md) and current
[slice report](../../reviews/WAVE6-M1-REPORT.md). All unfinished gates below
remain open; neither previous spike results nor these child cycles close them.

October 31, 2026, Asia/Amman. Branch `t3/production-gnome-wallpaper`, based on
`ad0f430` (local wave5). This is a **partial delivery** of the production-component
request, not completion of 6D-M1. The spike's successful lifecycle measurements
are not inherited as production acceptance.

## M1 resource slice follow-up (2026-10-31)

The importer now requires an explicit private cgroup v2 delegation, sets and
reads back 1 GiB memory / zero swap / 64-task ceilings, attaches the launcher
before exec, and kills/removes its child group on normal teardown. Missing
delegation refuses rather than using the former rlimit-only fallback. The
host's cgroup mount is read-only: live controller/pressure tests remain unproven.
This does **not** implement daemon presentation-plan IPC, enable authorization,
connect imported media to the renderer, or establish lifecycle acceptance.
No Mutter session or owner desktop was started/touched for this resource slice;
there is no new warning-baseline comparison or playback claim. See
[done/proven/not-proven report](../../reviews/WAVE6-M1-REPORT.md) and
[import boundary](../../WALLPAPER-IMPORT.md).

## Delivered surface

- Independent MIT OR Apache-2.0 extension source:
  `extensions/gnome/one-mk-wallpaper@one-mk.org`. No copied wallpaper-project code,
  decoder, network/media parser, test methods, arbitrary argv, unsafe-mode switch,
  FrameReady, image upload, or user-provided path channel in its D-Bus interface.
- `metadata.json` is the single exact-version qualification-list source, starting
  with **50.1**, the actual laboratory Shell. GNOME's own major-version validation
  remains on; the additional exact-version guard refuses all unlisted builds.
  There is no `unlock-dialog` session mode. Matching this list means only that
  the Shell build has laboratory evidence, **not** that playback/hardware passed.
- Exact section 4 wire XML lives in `bridge.xml`; Rust includes this same file.
  Hello `(u) -> (uua(iiiid))`, SetState `(u) -> ()`, Layout `(ua(iiiid))` and
  PauseHint `(u)` only; no properties. State/rate, nonzero serial, finite scale,
  checked edges, 16-monitor, 16384-side, 64-million-pixel and 4096-byte bounds.
  Empty output sets are valid. Layout changes coalesce for 100 ms.
- Spawned-PID plus UID authentication, one pending bounded credential lookup,
  first-Hello name binding, unicast signals, non-replaceable/nonqueued bus name,
  fail-closed name loss. Rust pins the bridge to Shell's PID **and** the inherited
  Wayland peer, subscribes before Hello, waits for the pause snapshot, and refuses
  owner changes/invalid signals. SetState duplicates coalesce in Rust.
- Source windows use connection ownership, DESKTOP before map, re-hide enforcement,
  one clipped clone per monitor, bounded retiring-window lifetime and replacement
  rate, geometry admission on damage, hidden-until-playing-and-damaged, stale
  watchdog, 2-second Hello and 5-second first-frame deadlines. All pause reasons
  hide first and terminate the renderer; uncover/unlock starts a fresh instance.
  Retry is 1/2/4 seconds, at most three starts per rolling minute; immediate unlock
  uses the **same** cap and never clears exhaustion. The latch survives extension
  rebase within a Shell session. Retry currently requires a fresh Shell session.
- CLI `one-mk wallpaper extension install|uninstall|status`, optional
  `install --renderer /absolute/path`. Sources are embedded in the CLI; installation
  requires an existing renderer binary. Only explicit install copies into
  `$XDG_DATA_HOME/gnome-shell/extensions/UUID`, or `$HOME/.local/share/...`.
  It never enables; it prints `gnome-extensions enable one-mk-wallpaper@one-mk.org`.
  Install refuses an occupied path. Uninstall refuses enabled, unmanaged, modified
  or extra-file trees rather than deleting user content. Pristine managed trees
  are fully removed. GNOME enabled-extension settings are never changed by it.
- Status distinguishes installed, enabled, qualified and restart-limit-reached.
  Unreachable Shell means **unknown**, not false. Doctor prints the extension state
  and the literal **restart limit reached** when the Shell-owned runtime snapshot
  says so. Snapshots carry Shell's unique owner so an old session is ignored.
  Import and `doctor --runtime` dispatch in `main.rs` remain unchanged.
- Release workflow has a separate **non-publishing** component artifact with
  `libexec/one-mk-wall`, extension source beneath `share/gnome-shell/extensions/`,
  licenses and SHA256SUMS. It is not attached to a GitHub release or auto-installed.
  `scripts/package-wallpaper-gnome.sh` reproduces packaging without installation.

## Explicitly unfinished — required before the request can be called complete

1. **Owner-authenticated daemon presentation-plan IPC and subscriptions are not
   implemented.** The production bridge's authorization stays false; no user-facing
   method or arbitrary argv override can turn it on. Enabling the extension alone
   therefore leaves it dormant, even if wallpaper policy is enabled.
2. **The production renderer is not connected to a qualified media/scene pipeline.**
   `one-mk-wall --present=gnome` explicitly refuses before opening a surface or
   decoder. `gnome-bridge` builds the production client library, not a fixture
   disguised as playback. The fixture/risk probe and conformance executable remain
   separate features/examples and are not packaged in the component artifact.
3. Source-window presentation, real damage/replacement, coverage, lock/unlock,
   power/battery/manual restrictions, display-power-off notifications, serial-wrap
   re-handshake, current/latest pending signal coalescing and failure re-probes
   still need production end-to-end integration/qualification. The current bridge
   rejects serial exhaustion until re-enable instead of transparently restarting
   the session; the client signal queue is bounded to 16 rather than a single
   latest slot. GPU-specific limits and physical/HiDPI import are not probed.
   Do not silently promote these partial implementations to section 4 compliance.
4. ~~Teardown uses immediate force-exit and async reap, not the proposed cooperative
   SIGTERM/two-second escalation.~~ Resolved in step C (SIGTERM, SIGKILL after 2 s). The cap is observable, but a user-facing explicit
   Retry operation is not delivered. Cold-install recognition without logout/login
   is not qualified. No on-host system or extension validation setting was changed.
5. Whole-runtime licensing/sandbox/decoder package acceptance, Windows/macOS native
   host acceptance, physical display quality and power are still outstanding M1
   work. A dry-run artifact is not a shippable optional playback package.

## Reproduction and evidence boundary

Use only the checked-in headless supervisor, not a live session:

```sh
cargo build --offline -j 2 -p one-mk
cargo build --offline -j 2 -p one-mk-wall --features gnome-bridge --example bridge-conformance
python3 docs/spikes/gnome/qualify-component.py
```

The supervisor creates private HOME/XDG directories, a private session bus and
private system bus under the ignored worktree `target/gnome-component-qa` directory.
Only its test-only harness enables Eval. It copies the conformance executable to
the fixed renderer path in the **private** extension tree; it changes authorization
only through isolated Eval. None of those mechanisms exist in shipped source.
It exercises CLI install/uninstall with private XDG data, authenticated real Rust
calls, layout serials, unicast isolation, pause-bit rejection, state/version/type/
rate negatives, GC enable/disable, name squatting, the actual failed-spawn cap and
doctor output. Renderer children/session services and copied source trees are
cleaned in `finally`, including failed attempts. No surface/damage/decoder pass
is claimed by this test.

Normal-session warnings are compared, including stack records, against the
unchanged M1a-2 baseline; deliberate version-refusal errors are retained in a
separate negative-test log. The version negative removes the lab version from
the private extension's in-memory list: it tests refusal logic, not a second
actual GNOME version. Small final logs/results and validation are retained in
`component-evidence`; generated binaries and temporary trees are not evidence.

## Estimate and owner acceptance

Keep **16–26 engineer-weeks remaining for M1, 74–118 total**, low confidence.
This checkpoint does not justify reducing those estimates: the authorization,
production media integration and native/physical gates dominate. Re-estimate after
the integrated production renderer passes the same six stress runs and matrix as
the spike, plus new protocol/CLI and real power tests.

The owner should not enable this as a working wallpaper yet. After those gates are
implemented, test explicit cold install/enable/disable/uninstall and logout/login
on their exact GNOME build, DING/other extensions, lock/unlock, coverage with several
windows, display off, AC/battery/manual pause interactions, fractional scaling,
rotation/hotplug/replug, suspend, crash/hang cap and Retry, permissions, real frame
damage and GPU/power behavior. Keep the native static background untouched and
verify cleanup/no focus or input theft. No owner-machine result is inferred from
the private headless session.
