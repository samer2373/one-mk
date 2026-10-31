# 6D-M1 step C evidence: lifecycle and restoration in the private GNOME 50.1 harness

October 31, 2026 (Asia/Amman). Script: [`qualify-m1-lifecycle.py`](../qualify-m1-lifecycle.py)
(seed 20261031, 100 sequences, all directed proofs). The qualifying run passed end
to end (`result.json`: `"passed": true`; console output `qualify-run.log`; per-step
log `lifecycle.log`). Binaries and sources are pinned in `source-sha256.txt`
(laboratory builds, `bench-opt` with debug assertions). Production authorization
stayed false throughout.

## Setup (all private)

- Headless GNOME Shell 50.1 / Mutter with **three** virtual 1920x1080 monitors,
  private session and system buses (`isolated-session.sh`), no X11, fake GDM for
  the lock screen, and `client/fake_logind.py`: a stand-in that owns
  `org.freedesktop.login1` on the private system bus only and emits
  `PrepareForSleep` on request. Nothing is actually suspended.
- The owner's desktop, X servers, clipboard, real session bus and
  `~/.config/one-mk` were never used. Every test process ran with `env -i`,
  throwaway HOME and runtime directories, null backend, no discovery/notify.
- cgroup delegation: this run's **own** transient user scope
  (`systemd-run --user --scope -p Delegate=yes --unit=onemk-stepc-lab`, talking
  only to the user manager's private socket; holder process moved to a `holder`
  leaf, `lab` child with cpu/memory/pids). Nothing was created in any
  pre-existing slice; the scope was stopped afterwards.
- Media generated locally (as step B plus a GIF with 0.4 s / 2.6 s frames),
  imported by the real sandboxed importer.
- A generated "user wallpaper" is set in the private Shell (`picture-uri`), and
  native baselines are captured for all nine layouts (`shots/user-wallpaper-*`).
- Canvas `-1920,0 7680x2700`: an absent computer's slice on the left, room for
  every committed local layout; this computer's origin at canvas `(0, 540)`.
- Laboratory authorization: private Eval, plus a test-only hook that
  re-authorizes the bridge instance the Shell re-creates on unlock, re-enable
  or restart (a stand-in for the step D authorization channel).

## Proven

1. **Layout change and hot-plug** (GIF exact on every pixel at scale 1, at block
   centres when scaled; VP9 PSNR; still exact): monitor added, removed, scale 2,
   fractional 1.25, order swapped, stacked, one monitor rotated (1080x1920) and
   back. The same renderer process re-plans each time: the Shell hides the
   layer at `monitors-changed`, the renderer fetches a fresh authenticated plan
   for the new serial and replaces its window. Reveal of verified slices after
   the change: 111 changes, median 0.57 s, max 1.58 s (design target 2 s).
   Immediately after each change the layer was hidden or already showed a
   fresh window of the new extent (structural check); screenshots then showed
   the user wallpaper (114) or correct new slices (5), never old slicing.
   A layout outside the committed canvas is static (paused plan, user wallpaper,
   no worker), the renderer stays alive past the 2 s replacement deadline, and
   a fitting layout plays again. 23 bursts of three changes 150 ms apart
   converged to verified frames (max 4.7 s).
2. **Lock/unlock** (real screen shield, fake GDM): at lock the extension is
   disabled by the unlock-dialog mode; the renderer gets SIGTERM and exits by
   itself (59 locks: median 97 ms, max 208 ms), its worker and cgroup removed by
   the renderer; while locked there is no bridge, renderer, decoder, cgroup or
   timer and the lock screen shows no frame. Each unlock starts a fresh renderer
   showing verified advancing frames (median 1.07 s, max 2.6 s; target 5 s).
   Six lock cycles in about a minute do not consume the restart cap.
3. **Suspend/resume** (`PrepareForSleep`): with and without the Shell's
   lock-on-suspend, the presentation is hidden and the renderer exits before
   sleep (68 sleeps: median 28 ms, max 49 ms), no worker/group/timer remains, the
   screen is the user wallpaper while asleep, and after wake a fresh renderer
   shows verified advancing frames (median 0.91 s, max 1.8 s). No frame from
   before sleep is shown again (new process, hidden until fresh damage).
4. **Restoration**: after daemon `one-mk stop`, SIGTERM, SIGKILL, consent
   revoke, `wallpaper disable`, bare renderer SIGKILL, extension disable,
   extension uninstall (GNOME `UninstallExtension`) and GNOME Shell SIGKILL +
   restart, the screenshot equals the user's own wallpaper (mean error 0.0 in
   every case), `picture-uri` is untouched, and no process of ours (by
   executable or by membership in the delegated cgroup) and no cgroup remains.
   After a Shell SIGKILL the renderer and worker ended on their own (6 kills,
   max 52 ms) before anything cleaned up for them. A bare renderer SIGKILL
   leaves one empty, process-free group, which the next renderer reclaims at
   start (11 times, `event=reclaimed stale_playback_groups=1`).
5. **Stills without keepalive**: the renderer announces bridge state 4
   (holding). A still stayed revealed 12 s, unchanged, with no Shell watchdog
   source and **zero** presenter-thread wakeups (the renderer main thread had
   27 context switches: the 2 s authority heartbeat, kept by design). A GIF with
   2.6 s frames alternates Holding/Playing and is never stopped.
6. **Restart cap**: Shell-chosen stops are credited back; renderer crashes still
   count: two SIGKILLs are followed by fresh verified renderers (2 s, 4 s), the
   third within a minute latches the cap and leaves the user wallpaper.
7. **100 randomized sequences** (random item, 2–4 random events, random
   teardown): 119 layout changes, 23 bursts, 59 lock, 40 suspend, 28
   suspend-with-lock, 21 renderer kills, 33 plan pause/play, 25 extension
   disable/enable, then 19 stop, 16 SIGTERM, 14 SIGKILL, 11 revoke, 13 disable,
   13 bare renderer kills, 15 extension disables, 10 uninstall + new Shell and 6
   Shell SIGKILL + restart. Every event ended in verified frames or the expected
   static wallpaper within its bound, every teardown restored the user
   wallpaper with nothing of ours left. 18 Shell sessions in total.
8. **Warnings**: zero new Mutter/Shell warning records against the unchanged
   M1a-2 baseline across all 18 sessions. A control phase with nothing of ours
   enabled showed what the harness itself causes: the Shell's TimeLimitsManager
   reacting to the stand-in logind (`Failed to update user state`, `Failed to
   inhibit suspend: ... UnknownMethod`) and an evolution factory losing its bus
   when the Shell is killed; those identities (223 records) are reported
   separately in `result.json`. Two `evolution-addressbook-factory` records at
   Shell kills (the control saw the calendar factory's identical message) are
   listed as other session services, not Shell/Mutter, never ours.
9. **Native cgroup tests** (bonus): the importer's two ignored kernel tests
   (`delegated_kernel_limits_attach_and_cleanup`,
   `delegated_kernel_memory_and_task_pressure`) passed against a child of the
   same private delegation (`logs/native-cgroup-tests.log`), then removed.

## Found and fixed during this step

- A late damage of the pre-layout window re-marked it fresh, so frames sliced
  for the old layout could be shown on the new one (seen as a visible layer
  over a not-yet-fresh window). Windows adopted under an older layout serial
  are never revealed again, and the layer hides whenever no fresh window is
  behind it; an unchanged logical layout is not a layout change.
- A spurious `EAGAIN` from the Wayland read was fatal: it killed a renderer
  during a layout change. Now ignored.
- The restart cap counted Shell-chosen stops: four lock/unlock (or cover and
  uncover) cycles in a minute latched "restart limit reached" for the session.
- The 5 s start deadline stayed armed after the first healthy frame and killed
  a renderer replacing its window for a layout change; a plan-paused renderer
  was failed by it too.
- A still sends one commit, and Mutter reports its first damage before the
  window has geometry: without the 2 s keepalive the still was never revealed.
  The empty-geometry damage is now re-checked (bounded, 40 x 25 ms).
- Stops were an immediate SIGKILL, which left the worker's cgroup behind;
  now SIGTERM with SIGKILL escalation after 2 s.

## Run history (honest)

Debug iterations ran with fewer sequences. Qualifying attempt 1 stopped at
sequence 10 on the harness's lossy-frame identification margin (VP9 at scale 2,
PSNR 44 dB, next frame 2.2 vs 1.1 mean levels); the margin rule became a ratio.
Attempt 2 (`logs/attempt2-result.json`) passed every event of all 100 sequences
but failed the warning gate on three `evolution-addressbook-factory` records at
Shell kills; the gate was changed to gate Shell/Mutter records and report other
services. Attempt 3 (`logs/attempt3-result.json`) failed at sequence 97 on a
harness bound that included the 3.5 s verification wait for a 2.6 s GIF frame
(the reveal itself was within bound); the bound now applies to the reveal. No
product code changed between attempts 1 and the qualifying run.

## Not proven / limits

- Not production: laboratory features, private Eval authorization and the
  re-authorization hook (step D). After a daemon stop the still-authorized lab
  Shell retries the renderer, which fails without an authority and counts
  toward the cap; production authorization must withdraw instead.
- Suspend is simulated by the signal only: no real sleep, frozen processes,
  GPU/DRM resume or clock jump. Lock is the screen shield without a real GDM or
  authentication. Hot-plug is virtual monitors only (one mode each, so
  "resize" means scale or rotation); no physical outputs, mixed scales, HiDPI
  sampling (buffers are logical) or rotation of the presented content.
- A layout outside the committed canvas pauses all local outputs (not per
  output); re-committing the canvas is 6D-M2.
- "Shell restart" is a SIGKILL plus a fresh private session; the user's
  wallpaper setting (memory GSettings) is re-applied by the harness, as dconf
  would. In-place Shell restart does not exist on Wayland.
- One machine; timings are from a loaded developer box. No power or
  long-duration measurement.
