# 6D-M1 step B evidence: real playback in the private GNOME 50.1 harness

October 31, 2026 (Asia/Amman). Script: [`qualify-m1-playback.py`](../qualify-m1-playback.py)
with [`m1_playback_media.py`](../m1_playback_media.py). Binaries: laboratory builds
(`one-mk` with `wallpaper-m1-lab`, `one-mk-wall` with `m1-lab`, `one-mk-wall-import`),
profile `bench-opt`, hashes in [`source-sha256.txt`](source-sha256.txt). The run
passed end to end (`result.json`: `"passed": true`); `qualify-run.log` is its
console output. Production authorization stayed false throughout.

## Setup (all private)

- Headless GNOME Shell 50.1 / Mutter, two virtual 1920x1080 monitors, private
  session and system buses (`isolated-session.sh`), no X11. The owner's desktop,
  X servers, clipboard, session bus and `~/.config/one-mk` were never used.
- Media generated locally only: GIF, APNG and lossless animated WebP of a
  576x108 pattern (10 frames, 100 ms), a 1152x216 still PNG, VP9 and AV1 WebM
  of a 960x180 pattern (30 frames, 30 fps), and an 8 s 1920x1080 30 fps VP9
  clip from GStreamer's `videotestsrc` (smpte, moving). Every pattern frame is a
  pure function of its index: thirds are red / green / blue dominant, a band
  carries a per-frame colour, a lower band the frame number in binary.
- All seven were imported by the real importer, `one-mk wallpaper import`
  (fixed sibling helper, bubblewrap + external seccomp + fd-pinned 32-file lab
  runtime + a fresh delegated cgroup v2 group with 1 GiB / no swap / 64 tasks).
- The cgroup delegation was an empty directory created for this run inside the
  T3 Code agents slice of the user's systemd hierarchy (the only user-owned
  place with the controllers available), with `import/` and `play/` children;
  all of it was removed afterwards. This is a laboratory delegation, not a
  production provisioning design.
- Daemon plan: canvas `-1920,0 5760x1080` with this computer at origin `0,0`,
  so the left third (`-1920..0`) is reserved for an absent computer;
  monitor slices `0,0` and `1920,0`, both 1920x1080.

## Proven

1. **Imported media decode in the sandboxed worker and present on both
   monitors.** For GIF, APNG, animated WebP, still PNG, VP9 and AV1, ten
   screenshots each (one for the still) were compared with crops computed from
   the generated frames using the renderer's own cover mapping:
   - GIF, APNG, still PNG: **every pixel below the top bar on both monitors
     exactly equals** the expected slice of the identified frame.
   - Animated WebP: every pixel within **one level**; the one-level difference
     is the image crate's compositing of WebP frames after the first, measured
     independently by decoding the file with the image crate (max channel
     difference 1 versus libwebp) — a decoder property, not presentation.
   - VP9 and AV1 (lossy, 4:2:0): PSNR >= **34.0 dB** against the original
     frames at every block centre; the correct frame wins by >= 10.6 mean
     levels over the next best.
   - Both monitors always show the **same frame**, each its own slice; the
     crop of the reserved absent slice differs by >= 33 mean levels and **no
     red-dominant (absent-third) pixel** appears anywhere on screen.
2. **Frames change over time and loop.** Identified frame indices advance in
   real time (each step between screenshots matches the elapsed time modulo
   the loop) and wrap: e.g. GIF `[2, 4, 6, 8, 1, 4, 8, 2, 6, 1]`, VP9
   `[7, 12, 18, 26, 4, 14, 25, 7, 20, 4]`.
3. **Loops are seamless (hard cut, no stall).** The renderer logs the gap
   between the last frame of a loop and the first of the next. All 28 logged
   seams were within **-0.40..+0.26 ms** of the nominal frame duration
   (100 ms images, 33 ms video); the VP9/AV1 worker loops by a flushing seek
   inside the sandbox, and the native test proves every loop decodes to the
   same bytes. One late frame (re-anchored) was counted across all runs.
4. **Still image is stable.** Identical screenshot 4.5 s later (longer than the
   Shell's 3 s damage watchdog), still revealed, and the decode worker exited
   right after its one frame (no decoder for a still).
5. **Pause in the plan pauses presentation.** Daemon `pause` → new plan with
   `paused=true` → renderer reports `Paused` first (Shell hides the layer),
   then destroys the worker (no playback cgroup left) and the surface (no
   window). The screen equalled the native background exactly for 3 s and the
   renderer used **0 CPU ticks** meanwhile (no paused timer or child); `play`
   resumed real, verified frames.
6. **1080p30 VP9 resources** (30 s window, optimized lab builds, software
   decode, 2 x 1080p monitors): renderer **13.1 %** of one core, sandboxed
   worker **42.6 %**, gnome-shell **8.5 %**; renderer RSS **38.8 MiB**
   (17.7 MiB anonymous, 18.5 MiB shared surface buffers), worker cgroup memory
   **67 MiB** (peak), worker processes' RSS 80 MiB. 961 frames presented since
   start with **0 late, 0 starved**, no cgroup CPU throttling under the
   two-CPU `cpu.max` ceiling. First frame 78 ms after the plan.
7. **Teardown and leftovers.** Renderer SIGKILLed by the Shell while playing:
   its 3 sandboxed worker processes died with it, and the next playback
   reclaimed the empty stale group. Clean exits leave no playback group.
8. **100 start/stop cycles of real playback** rotating the seven imported
   items (14–15 each) and five teardowns (consent revoke, disable, daemon
   `one-mk stop`, SIGTERM, SIGKILL; 20 each). Every cycle: plan admitted,
   first frame, Shell revealed the layer, a screenshot verified the item on
   screen (exact/PSNR slice check, or media-not-background for the 1080p
   clip), then the renderer exited (median 15.6–83.1 ms, max 84.8 ms), every
   worker died, no playback group and no laboratory process remained.
   First frame over all 111 renderer runs: median 50 ms, max 243 ms.
9. **Zero new Mutter/Shell warning records** against the unchanged M1a-2
   baseline (14 records, all known), across the whole run.

## Found and fixed during this step

- The production extension stopped every real renderer at its first damage:
  Mutter 50.1 emits that damage before the DESKTOP window has buffer geometry
  (0x0), which the size check treated as a wrong surface. Step A never
  created a window, so this path had never run. Empty geometry is now "no
  content yet"; wrong non-empty sizes are still refused.
- The v1 bridge cannot tell the Shell that a still (or a long frame) expects
  no damage, so the 3 s watchdog would stop it. The renderer re-commits the
  unchanged frame every 2 s in that case (a 0.5 Hz wakeup while a still or a
  long frame is shown; none while paused). The design's frame-deadline
  signalling remains to be done.
- Software colour conversion of full 1080p frames cost ~30 % of a core in the
  worker; video frames now stay I420 and the renderer converts only the pixels
  it shows (BT.601/BT.709, limited/full range from the negotiated caps).

## Not proven / limits

- Not production: features are off in release builds, the artifact gate now
  also refuses laboratory playback in the release renderer, plain
  `--present=gnome` refuses, Private Eval authorizes the bridge.
- Cover fit only (no contain/stretch/tile), nearest-neighbour CPU sampling,
  one shared-memory surface; no GPU, HiDPI/fractional scale, rotation, HDR or
  physical display. Two equal virtual monitors on one computer; no second
  computer actually presents the reserved slice and there is no cross-computer
  clock/epoch (resume after pause restarts the loop at frame 0).
- Layout change, hot-plug, lock/unlock, suspend/resume and restoration are
  step C; only the plan-pause and teardown paths were exercised here.
- The 1080p numbers are one machine, software VP9 decode of a synthetic clip;
  no power, wakeup or long-duration (hours) measurement. The design's proposed
  budgets (e.g. 4K30 <= 10 %) are not met by software decode and not claimed.
- The worker reads the source through the descriptor the renderer hashed; a
  same-user writer can still change the file afterwards (outside the threat
  model). Same-user `/proc` and cgroup access are outside it too.
- Windows/macOS: untouched.
