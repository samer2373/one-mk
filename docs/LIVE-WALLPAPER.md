# Live wallpaper implementation and acceptance record

Implementation branch: `wallpaper-review-fixes`, including `3adcf763` from
`polish-glass-friction`. Evidence recorded on 2 November 2026. This is an
implemented X11 path and compile-checked Mac/Windows backends, **not completed
three-platform acceptance**. There is no defensible promise of zero performance
impact: the software-decoded Linux measurements below are still material.

## Using the window

Open **Wallpaper**, choose or drop a video or picture, and choose Fill, Fit or
Stretch. The preview uses the same reserved canvas and monitor offsets as
Layout. The existing Play on switches select participating computers. Video
rows show preparing, playing, a pause reason, or a plain error for each computer.
Pause retains a video selection; Turn off stops playback and restores any native
picture wallpaper changed by oneMK. Pictures do not start a player process.

Power-user commands remain `one-mk wallpaper set FILE`, `pause`, `play`, `off`,
and `status`; the normal window needs no commands or configuration editing.
Videos depend on the local system decoder on each computer. PNG, JPEG and WebP
pictures are decoded once at selection/slicing time. HEIC uses the native Mac
decoder or an installed Windows WIC decoder; Linux HEIC is not implemented.
Animated pictures are not a video replacement workflow.

## Implementation

- `crates/wallpaper-player`: shared geometry, import limits, picture slices,
  playback status and cache contracts; the separate native video helper.
- `crates/one-mk/src/live_wallpaper.rs`: controller-only preparation before
  hashing, SHA-256 cache, authenticated distribution, receiver readiness,
  clock samples, native picture application, child ownership and persistence.
- `crates/ctl/src/wallpaper.rs`: shared Layout/status coordinates, reserving
  offline computers' space. The existing scale-metadata limitation remains.
- `crates/app/src/app/wallpaper.rs`: minimal changes to the existing page:
  picture file filters/hint, video-only pause controls and per-computer notes.
  Shared style/chrome/glass/widget implementation files are untouched.

Transfers reuse the pinned QUIC file path and `LIVE_WALLPAPER` capability
**4096**, not typing's 1024. No protocol version or capability bit was bumped.
Only `<64 lowercase hex characters>.video` cache assets are admitted, at most
4 GiB, and only when that authenticated peer's current or pending plan names
the hash. PlanReady acknowledgment installs admission before the sender opens
the transfer. **Update participating computers together**: older peers do not
understand PlanReady and sharing times out with an upgrade message. Older
helpers also cannot apply picture bundles. Status readers retain serde defaults.

The inbox serializes media arrivals, limits accepted offers to one per two
seconds, removes unreferenced hashed assets at startup/before admission, and
limits media to twice the larger current/incoming asset, with an 8 GiB total
wallpaper-cache ceiling including extracted pictures and partial transfers.
Verified SHA-256 results are cached by size, modification time and inode/file
identity (plus Unix change time), checked again after hashing. Cached media
skips bytes and repeat hashes; a changed/replaced file invalidates verification.
An identical pending/running plan skips probing, poster creation and player
replacement, and re-sends existing playback status. Connection/Caps changes
send the existing plan only to newly capable peers; they do not restart the
controller or other running peers. Actual canvas/selection changes still do.

### Controller-only video preparation

Before hashing/copying a newly chosen video, probe its dimensions, duration and
frame rate. Compute each computer's logical monitor union at its largest known
scale, then the largest per-computer pixel need. Keep aspect ratio, never
upscale, use even dimensions, and target approximately 0.045 bits/pixel/frame
(with bounded bitrate). Attempt preparation when dimensions exceed that need,
source bitrate exceeds the target by 1.5 times, or frame rate exceeds 60.

- Mac: AVAssetReader/AVAssetWriter with a required VideoToolbox hardware encoder.
- Windows: Media Foundation source reader/sink writer with hardware transforms;
  refuse an encoder unless its transform reports a hardware activation URL.
- Linux: only installed `vah264enc` or `vaapih264enc`. No software encoding
  fallback is launched when these are absent or fail.

All prepared outputs request **H.264 High, video-only MP4, moov before media
data, at most 60 fps**. H.264 is deliberately always selected: this change does
not add HEVC encoding or claim an all-peers HEVC capability negotiation.
The existing status message shows **Preparing video… N%**. An unavailable or
failed hardware transcode keeps the original and gives a plain explanation.
Successful output is probed, SHA-256-addressed and retained in the existing
cache; temporary output is removed. Receivers never transcode.

Hardware encoding, rotation/color correctness, faststart layout and profile
must still be checked on real Mac/Windows hardware. In particular, some Windows
hardware MFTs may not expose the activation URL queried here; that safely falls
back to the original rather than silently using a CPU encoder.

### Native pictures

The controller uses `Canvas::slice` and the same Fit/Fill/Stretch math to make
bounded per-monitor PNG slices. One SHA-addressed bundle per computer travels
through the existing distribution path; receivers validate count, byte limits,
dimensions and trailing data before applying it. Bundle indices avoid repeated
slicing during ordinary reconciliation. Source and current/previous bundles
are retained alongside the video cache.

- Mac: `NSWorkspace` per-`NSScreen` image URLs, on the main queue. Save the old
  URL, scaling/clipping options and fill color, keyed by the stable
  `CGDisplayCreateUUIDFromDisplayID` UUID rather than transient screen numbers.
- Windows: `IDesktopWallpaper` monitor device paths matched by rectangle, with
  the previous URLs, wallpaper position and background color saved for restoration.
- Linux: GNOME-compatible `gsettings` picture-uri, picture-uri-dark and spanned
  positioning. Assemble local slices into one spanned image where per-monitor
  setting is unavailable; save and restore all three settings. This also works
  independently of the unavailable GNOME Wayland video backend.

Picture decode, resize, PNG encoding, snapshot I/O and Linux settings commands
run on a serialized blocking worker, not the daemon runtime thread or under
the wallpaper state mutex. Unlock/display wake does not reapply an unchanged
picture. Writes use a temporary file plus rename; Mac/Windows slices get new
filenames so native wallpaper caches notice replacements. Main-queue Mac work
is confined to screen queries/native settings; Null-backend daemon runs also
pump the main queue instead of deadlocking a synchronous dispatch.

Newly observed screens merge into the original snapshot before mutation.
Turn off restores connected screens independently. Unplugged screens, a
different active Space, or failed native calls leave a pending snapshot and
its referenced PNGs intact; reconnect/return to that desktop and repeat Off.
Snapshots survive restart. Cleanup removes only unreferenced pictures, and
complete restoration removes the snapshot and remaining oneMK PNGs. The prior
and proposed Mac/Windows URLs are both retained across a failed native swap.
Legacy Mac snapshots keyed by screen number cannot safely recover a historic
UUID; unmatched entries are retained, not silently discarded.

**Native modes deliberately refused rather than damaged:** Mac dynamic/HEIC
originals, aerials, per-Space settings, missing/unreadable wallpaper stores and
opaque store payloads cannot be faithfully round-tripped through NSWorkspace.
The conservative store check can also refuse ordinary static configurations
whose settings are opaque. These fail before native mutation; this is not
dynamic/aerial/other-Spaces support. If the user changes wallpaper/Space during
a oneMK picture session, Mac Off retains unmatched entries rather than
overwriting that unrelated desktop. Windows configured/paused slideshow mode
is refused before changing it; slideshow configuration is left untouched.
Windows empty-image/solid-color backgrounds restore the original empty URL
and color; the API does not expose a separate readable background-enable flag
(`DSS_ENABLED` describes slideshow status). KDE/other Linux desktops without
GNOME settings need their own adapter.

### Readiness, replacement and clocks

Keep the previous helper alive while preparing a new selection. Release it
only after the new helper reports a display-ready frame. This implementation
uses a **cut**, not a 300 ms crossfade (the requested alternative), avoiding a
permanent second decoder. Temporary old/new decoders overlap during preparation.
Failed preparation leaves the previous visible helper alive until another
successful replacement or Off. Launch identities prevent an intentionally
stopped or superseded child from overwriting current readiness/errors.

Mac windows start black with alpha zero, become visible only after every
`AVPlayerLayer.readyForDisplay`, and retain the old outputs until then. Windows
checks the raw tick HRESULT for **exactly S_OK**, not S_FALSE, before either
poster readback or GPU TransferVideoFrame/Present and DwmFlush; X11 gates on GStreamer preroll,
initial-position preroll, raise and XSync. These are API readiness/submission
signals, **not proof of physical compositor scanout or a no-flash guarantee**.
Mac and Windows report a plain failure if the first playing frame takes more
than 40 seconds. Native hotplug replacement retains old outputs until ready.

`Status.computers` records preparing/ready/paused/error. Receivers forward
helper readiness or a plain probe/decoder error to the controller; the Play on
row reads those records. An older receiver that cannot acknowledge remains
preparing. Feedback is keyed by source hash, not a separate plan revision;
very rapid same-source Fit/selection changes can momentarily accept stale
feedback. Physical paired-machine delivery/ordering still needs acceptance.

Playback uses one canvas and epoch. A receiver completes five midpoint clock
samples with the new controller **before first video start**; unavailable
samples fail preparation rather than launching at zero offset. Saved receiver
video selections wait for a new plan/sample batch after daemon restart.
The best-RTT batch is refreshed after 30 seconds of active playback. Within
40 ms there is no correction; otherwise nudge 0.98/1.02, **never periodically
seek**, including large drift. Initial start/resume and loop boundaries may
reposition. Network asymmetry, variable refresh and rate-only recovery from
large clock changes prevent a frame-exact guarantee.

### Idle and pause behavior

Off adds no wallpaper timer or polling loop to the daemon. Lock, sleep, user
pause and shutdown stop/reap its helper; parent EOF also ends it. Backend
battery/low-power/coverage holds pause the decoder, not merely hide a window.
Independent holds do not cancel one another. Receiver clock work sleeps on a
notification while paused/preparing instead of retaining its refresh timer.

Availability is cached once per process. Input-side Layout reconciliation
does not build a full daemon/wallpaper status, and returns immediately when
there is no plan or pending Off distribution.

Mac drains NSEvents as well as servicing its CFRunLoop; off-main power
notifications dispatch to the main queue before changing main-thread state.
Lock still reaps the helper (rather than claiming a zero-cost paused decoder).
Windows owns one latency wait handle per swapchain, closes it on rebuild/drop,
and uses foreground/move-size-end/minimize/top-level object events instead of
the system-wide cursor/caret LOCATIONCHANGE hook. Coverage is evaluated only
on relevant events; battery/saver state comes from power notifications, not
per-frame polling. Linux subscribes only to root EWMH properties, recomputes
coverage only for active-window/stacking changes, and reads power state only
on power-supply events. It does not subscribe to client title/user-time changes.
GStreamer sink event/expose polling is disabled; oneMK handles Expose on its
own wallpaper windows and explicitly repaints the retained frame on demand.
Fully paused loops have no timeout.
Active clock correction runs about every two seconds; Windows presentation is
paced at source fps (capped at 60), not a busy presentation loop. OS event
delivery may still cause work while paused; literal zero CPU/GPU everywhere is
not established by these structural changes or the finite measurements.

## Platform evidence and limits

| Platform | Implemented path | Evidence / outstanding work |
| --- | --- | --- |
| macOS | One borderless non-activating window per NSScreen, desktop-icon level minus one, join-all-Spaces + stationary + ignores-cycle; one AVPlayerLayer per window sharing one muted AVQueuePlayer/AVPlayerLooper. Each clipped root shows its screen-local whole-image layer rect with the screen backing scale. Screen notifications rebuild outputs; occlusion pauses only when every output is not visible. Lock/sleep/battery/Low Power Mode remain; no display-sleep prevention; automatic waiting for stalling is enabled. | ARM Apple target compile-check only. Same-player multi-layer rendering, separate Spaces, mixed DPI, native stills/import, readiness cut and hardware cost require real Mac acceptance. No per-frame CPU copies were added. |
| Windows | WorkerW children, one IMFMediaEngine/D3D11 decode, per-monitor GPU TransferVideoFrame, flip-sequential double-buffered swapchains with frame-latency waits and source-fps pacing. WM_DISPLAYCHANGE/WM_DPICHANGED rebuild outputs; foreground rectangle coverage and shell presentation state hold playback; power notifications and battery-saver SystemStatusFlag hold it too. | MSVC compile-check only. Real first-frame display, wait-handle pacing, fullscreen/multi-monitor coverage, hotplug/DPI, native images/import and battery saver are unverified on Windows hardware. No per-frame CPU readback; poster generation still reads back once. |
| Linux X11 | One desktop/below/sticky window per monitor, one GStreamer decode/downscale before tee, bounded two-frame queues, per-slice crop/scale/XImage sinks and event-driven pause. | Xvfb seams, loops, coverage pause/resume and parent EOF verified. This is still CPU colorspace/crop/presentation, not a zero-copy GPU path. VA decode is preferred when a render device is visible. Legacy NVIDIA factories are not selected: their CUDA-memory negotiation failed the XImage path in a diagnostic run; a compatible GPU-download/presentation path remains work. |
| GNOME Wayland | Native pictures via GNOME settings; video unavailable | No production Shell extension is supplied. |
| KDE Wayland | Unavailable | No Plasma adapter. |
| wlroots / COSMIC / Hyprland | Unavailable | No layer-shell video backend or native picture adapter. |

Remaining limits include true per-monitor density transport: ctl conversion
still uses scale 1. The Mac layer itself uses NSScreen's real backing scale,
but controller import targets/picture slices can underestimate remote Retina
pixel needs. Helper-local hotplug rebuilds do not independently renegotiate the
global canvas with peers; daemon Layout reconciliation remains authoritative.
Integrated-GPU selection, GPU utilization and display-frame timing/drop counts
are not implemented. Linux programmatic window geometry changes that produce
neither active-window nor stacking updates can leave coverage stale until a
relevant WM event. The older Wave 6 importer/extension work remains lab-only
behind `wallpaper-m1-lab`, not the normal release interface.

## Linux measurement record

`scripts/test-live-wallpaper.py` generates system-GStreamer VP8 WebM, defines
two 640×360 XRandR outputs in Xvfb, verifies seam pixels, continued loop motion,
cover/resume, user-paused expose repaint and stdin-EOF teardown. The final
review-fix run on 2 November 2026 uses a debug helper and 30 fps software VP8.
CPU is user+system time; 100% means one logical core. The destinations are two
small slices, **not two physical 1080p/4K outputs**.

| State | Sample | CPU | Main / all-thread context switches |
| --- | ---: | ---: | ---: |
| 1920×1080 loop | 10 s | 11.2% | Not collected |
| 3840×2160 loop | 10 s | 26.6% | Not collected |
| Both outputs covered, idle | 10.02 s | 0.0% | 0 / 0 |
| Covered, unrelated client title/user-time updates at 20 Hz | 10.00 s | 0.0% | 0 / 0 |
| User-paused helper, idle | 10.02 s | 0.0% | 0 / 0 |

An initial review-fix measurement still found 400 all-thread context switches
per ten seconds with a sleeping main thread: the two GStreamer sink event
threads polled even with input events disabled. Disabling their expose polling
and handling own-window Expose events explicitly removed those measured wakes;
the paused-frame repaint check passes. Context-switch deltas from every
`/proc/PID/task/TID/status` are a **wakeup proxy**, not interrupt counts.
Zero CPU ticks in a short sample does not establish universal zero work.

`scripts/test-idle-wakeups.py` also passed with wallpaper Off, discovery disabled,
20 seconds settling followed by a 20-second sample. Main-thread voluntary
switches/second were:

| Peer state | Null backend | X11 backend |
| --- | ---: | ---: |
| Alone | 0.00 | 0.00 |
| Paired but offline | 1.10 | 1.05 |
| Connected | 3.95 | 4.20 |

Other measured daemon threads recorded zero context switches. The peer cases
still perform normal connection/retry work; these are not zero-wakeup daemon
claims. GPU utilization was **not collected**: Xvfb is not a physical GPU
presentation test. **Software 4K still does not meet a negligible-impact bar.**
Smoothness, GPU/energy cost and concurrent input latency require physical
desktop measurements, not an inference from average CPU or compile checks.

Raw local evidence: `/tmp/wallpaper-review-xvfb.log`,
`/tmp/wallpaper-shots/measurements.json`, `/tmp/wallpaper-shots/` (seam, loop,
paused-expose PNGs and xrandr topology), `/tmp/wallpaper-review-idle.json` and
`/tmp/wallpaper-review-idle.log`. These temporary files are not shipped assets.

## Validation

Use `CARGO_TARGET_DIR="$PWD/target-wallpaper-review-fixes"`, no more than four jobs,
and never `cargo fmt`. No fat-LTO build was performed. The private target
directory is removed at handoff. Checks on this branch:

- `RUST_TEST_THREADS=4 cargo test -j 4 --workspace`: passes, including 454 daemon
  unit tests, 16 wallpaper library tests and the off-idle integration test.
  A sandboxed focused attempt failed on denied private sockets/D-Bus; the
  unrestricted focused and workspace reruns pass. A subsequent full run hit
  the known-flaky X11 `devices_are_listed_and_input_says_which_one` event timeout;
  the exact test passed alone, then the full workspace was rerun unchanged.
- `cargo clippy -j 4 --workspace --all-targets -- -D warnings`: passes.
- `RUSTFLAGS="-D warnings" cargo check -j 4 --all-targets -p one-mk-wallpaper -p one-mk-app -p one-mk-ctl -p one-mk-proto -p one-mk-input`
  passes with `--target aarch64-apple-darwin` and
  `--target x86_64-pc-windows-msvc`. These are compile checks, **not native
  linking, execution, signing, notarization, encoder tests or GPU acceptance**.
  The daemon is intentionally not cross-checked.
- Regression coverage includes import thresholds/no-upscale/fps bounds,
  mixed-density origin invariance, old-peer defaults, native-picture bundle
  slicing/corruption/borders, readiness retirement, stale-child EOF and decoder
  error preservation; existing canvas/cache/distribution/lifecycle tests pass.
  New tests cover connection-only/duplicate plans retaining a child PID and
  acknowledging readiness, metadata-based verified-hash invalidation, initial
  clock sampling before any child starts, inbox admission/rate/cap accounting,
  exact S_OK vs S_FALSE, atomic picture readers, snapshot merge/partial restore
  and retention of current/unrestored/failed-swap PNG references.
- Xvfb seam/loop/pause/expose/EOF/CPU and daemon off-idle scripts pass.
- `git diff --check` passes. Packaging was not rerun for this branch; earlier
  wallpaper-live packaging evidence is not treated as new native acceptance.

## Dependency licences

No new lockfile packages were introduced. Additional direct helper dependencies
already in the workspace lockfile are block2 0.6.2 (MIT), dispatch2 0.3.1
(Zlib OR Apache-2.0 OR MIT), and libc 0.2.191 (MIT OR Apache-2.0). The image
0.25.10 dependency (MIT OR Apache-2.0) now enables PNG/JPEG/WebP, including
zune-core 0.5.3 and zune-jpeg 0.5.15 (MIT OR Apache-2.0 OR Zlib), and
image-webp 0.2.4 (MIT OR Apache-2.0). Checked against installed Cargo manifests.
Windows/objc2 features only expose more native API bindings; they do not bundle
codecs. Existing libloading is ISC, x11-dl is MIT, and objc2/Windows bindings
are permissively licensed as recorded in their manifests.

AVFoundation, VideoToolbox, Media Foundation and dynamically loaded GStreamer
are system components. **No FFmpeg, libmpv, x264 or x265 is bundled.** Only the
named VA hardware encoders are requested on Linux; installed system plugins
retain their own licences. Muro/Aura-style single-decode fan-out, event-driven
holds and desktop-window placement are techniques, not reused source. No Muro,
Aura, Hanabi or Lively source was copied or incorporated.

## Owner's Mac acceptance checklist

1. On a real Mac, install/link/sign the helper and daemon. Test H.264/HEVC
   chooser and drag/drop, invalid files, PNG/JPEG/WebP/HEIC and all three fits.
   Native HEIC, portrait video/image orientation and colors must be correct.
2. Test negative origins, horizontal/vertical arrangements, Retina/non-Retina
   and hotplug/DPI changes. Verify one window per NSScreen, all layers actually
   showing the shared player's video, correct seams, icons still clickable,
   separate Spaces, Mission Control and fullscreen apps on either screen.
3. Cover one screen (continue); cover every screen (decoder pauses). Combine
   lock, sleep, unplugged power and Low Power Mode; releasing one hold must
   not defeat another. Observe no wallpaper polling wakeups while fully held.
   Exercise off-main power notifications and repeated screen/occlusion changes;
   confirm NSEvent processing stays responsive and its queue does not grow.
4. Watch ten loop boundaries and rate nudges; replace short/long/different-fps
   clips repeatedly. Capture the readiness-gated cut at high frame rate: no
   desktop/black flash. Failed new playback must retain the previous frame.
   Repeatedly lock/unlock: new windows must remain alpha-zero until ready,
   without the former opaque-gray flash. Locked helpers should be absent.
5. Choose oversized/high-bitrate/120-fps clips. Verify preparation progress,
   actual VideoToolbox hardware encoder, <=60 fps, no upscaling, H.264 High,
   audio removal, MP4 moov-first and cache reuse on receivers. Disable encoder
   availability and verify a clear original-video fallback.
6. Set different native wallpapers/scaling/colors per display. Apply a shared
   picture, change Fit and selection, restart the daemon, then Off. Verify
   exact previous wallpaper/options restoration and **no player for pictures**.
   Use a safely inspectable static configuration; also confirm dynamic/HEIC
   originals, aerials, opaque settings and other-Space configurations are
   refused without native changes. Test Null-backend picture application for
   main-queue deadlock, display UUID survival across unplug/replug, a newly
   added screen, and partial Off retaining its snapshot/PNG until retry. Inject
   a failed native setting call during replacement and retry Off.
7. Record hardware/OS/codec/fps/dimensions, Activity Monitor CPU/GPU/energy and
   frame-time/drop evidence for 1080p/4K for at least one minute, then covered,
   locked, battery and Off. Confirm one steady-state decode per computer and
   no per-frame CPU image copies. Do not accept smooth/0% from a compile check.
8. Pair real peers. Verify preparing -> playing/error notes, copy/cache hits,
   seam/time, paused receiver clock silence, reconnect/offline reservation and
   Off while offline. Test an unsupported receiver codec and rapid same-source
   Fit changes. Remote Retina density transport remains an explicit open gap.
   Record player PIDs across connect/disconnect/Caps changes: existing players
   must stay alive, only the newly capable peer receives a plan, and duplicate
   plans re-report readiness without hashing/probing. Start a new controller
   with at least 20 seconds of clock skew and verify sampling precedes playback.

## Owner's Windows acceptance checklist

1. Build/link/run on Windows with the supported system decoders. Test WorkerW
   icon layering/clicks, native chooser/drop, invalid codecs and plain errors.
   Exercise both the old top-level WorkerW and Windows 11 24H2 child/layered
   Progman layouts; an unsupported shell must fail cleanly.
2. Test 100%/150%/200% DPI, negative origins, mixed refresh rates and monitor
   unplug/replug. WM_DISPLAYCHANGE/WM_DPICHANGED must replace outputs without
   flashing or duplicate windows; confirm source-fps flip-model presentation
   and latency-handle waits in a profiler, including minimized/covered states.
   Track kernel handles through long playback and repeated output rebuilds:
   one wait handle per swapchain, with no per-frame or rebuild growth.
3. Test a fullscreen foreground window, multiple covered monitors, shell
   presentation mode, lock/sleep, battery and battery saver independently and
   together. A visible monitor must continue; all covered must pause decoding.
   Fully paused playback must have no presentation timer or busy polling.
   Move the cursor/caret continuously, including oneMK-injected input, while
   paused: it must not trigger wallpaper coverage/power work. Unplug AC and
   toggle saver mode to verify notification-driven holds without frame polls.
4. Repeatedly replace videos, including decoder failures, and capture actual
   presentation. The old helper must survive until the new first frame is
   visible; then only one decode remains. Check ten loops and 0.98/1.02 nudges.
   Trace S_FALSE ticks before the first frame and between decoded frames: no
   premature Ready, black poster or duplicate TransferVideoFrame/Present.
5. Test oversized/high-bitrate/>60-fps and portrait sources. Confirm a hardware
   MFT, actual resizing/orientation, H.264 High, <=60 fps, no audio, moov-first
   and original fallback if the hardware URL check/encoder is unavailable.
6. Set different wallpapers per monitor and a nondefault wallpaper position.
   Test PNG/JPEG/WebP and HEIC with/without the OS decoder. Shared picture/Fit
   changes must use no player; Off after restart restores URLs and position.
   Include background color/solid-color mode, a screen added during the
   session, disconnected-screen partial restore and replug/Off retry. The
   pending snapshot and referenced PNGs must survive failures. Configured or
   paused slideshows must be refused without changing slideshow settings.
7. Use Task Manager/GPUView/ETW to record CPU/GPU/energy, video-decode engine,
   frame pacing and drops for one-minute 1080p/4K, all covered, battery saver
   and Off. Confirm GPU TransferVideoFrame and no per-frame CPU readback.
8. Repeat the real paired-peer status/cache/clock/reconnect/Off cases from the
   Mac checklist. Native hardware and real multi-computer acceptance are
   still required before claiming negligible cost or seamless playback.
