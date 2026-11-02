# Live wallpaper implementation and acceptance record

Implementation branch: `wallpaper-live`, based on `polish-glass-friction`.
Evidence recorded on 2 November 2026. This is a working X11 implementation and
compile-checked native backends, **not a completed three-platform acceptance**.
In particular, perfectly smooth playback, negligible resource use, Mac Spaces,
and real cross-computer playback have not been demonstrated.

## Using the window

Open **Wallpaper**, choose or drop a video, and choose Fill, Fit or Stretch.
The preview uses the video's poster and the same monitor offsets as Layout.
The switches choose participating computers and pause-on-battery behavior.
Pause retains the selection; Turn off stops playback. There is no importer,
extension-installation, consent or sandbox setup in the normal window/CLI.
Unsupported desktops show an explanation with playback controls disabled.

Power-user commands are `one-mk wallpaper set FILE`, `pause`, `play`, `off`,
and `status`. The window does not require commands or configuration editing.
Supported media depend on the local system decoder on **each** computer.
MP4/MOV H.264/HEVC and WebM are attempted without bundled codec libraries.
There is no explicit GIF or still-picture workflow yet.

## Implementation

- `crates/wallpaper-player`: shared geometry, clock, pause-reason and cache
  contracts, plus the separate `one-mk-wallpaper` executable. It has no daemon
  event loop or idle timer when wallpaper is off.
- `crates/one-mk/src/live_wallpaper.rs`: selected-video state, bounded native
  probe, SHA-256-addressed cache, authenticated peer distribution, clock
  samples, child ownership, persistence and reconnection.
- `crates/ctl/src/wallpaper.rs`: conversion of the shared Layout/status monitor
  coordinates to one reserved canvas. Offline computers retain their space;
  unpaired computers are removed on reconciliation.
- `crates/app/src/app/wallpaper.rs`: independent page using the current shared
  cards, buttons, pills and switches. Poster loading, chooser and drop handling
  are outside the style/chrome/glass/widget implementation files.

Transfers use the existing pinned QUIC file-transfer path with a dedicated
stream kind. The separate `LIVE_WALLPAPER` capability prevents sending new
control frames to older peers; the window reports that those peers need an
update instead of breaking their input connection. Only
`<64 lowercase hex characters>.video` is admitted, at most
4 GiB, and SHA-256 is checked before accepting a plan. A verified existing
cache entry is acknowledged without retransmitting media bytes. Current and
previous videos/posters are retained. Receiver-side native probing catches a
locally unsupported decoder before starting that receiver's helper. The
controller does not yet receive a decode-ready acknowledgement from receivers.

The controller publishes one canvas and start epoch. A receiver estimates
clock offset using round-trip midpoint samples (five per batch, best RTT,
refreshed every 30 seconds while enabled). Playback position wraps modulo the
duration. Within 40 ms no correction is made; 40–500 ms uses a 0.98/1.02 rate
nudge; only larger phase error seeks. Linux changes GstClock calibration
without periodic flushing seeks. Network asymmetry and differing display
refresh rates still prevent a frame-exact guarantee.

User pause, daemon lock/sleep events, off and daemon shutdown kill/reap the
owned helper; unexpected parent death closes its stdin and ends playback.
Backend battery/low-power/coverage holds pause the decoder without discarding
independent hold reasons. Off intent persists so an offline receiver is told
to turn off when it reconnects. No polling work is added to the off daemon.

The previous Wave 6 importer/authority/extension material is retained as
development history. Its commands and diagnostic surfaces are not the normal
release interface; importer dispatch requires `wallpaper-m1-lab`. Those
development binaries are not added to release archives.

## Platform evidence and limits

| Platform | Implemented path | Evidence / outstanding work |
| --- | --- | --- |
| macOS | AVQueuePlayer + AVPlayerLooper, muted, bounded read-ahead, no display-sleep prevention; borderless non-activating desktop-icon-level-minus-one window, one AVPlayerLayer; lock/sleep/low-power/IOPS battery and occlusion holds; display-change notification | Both Apple target architectures compile-check. **One spanning window, not one window per NSScreen.** Separate Spaces, mixed-DPI physical output, occlusion, loop smoothness, hardware-decoder use and resource cost need native acceptance. |
| Windows | WorkerW child windows, one IMFMediaEngine with a D3D11/DXGI device manager, per-monitor TransferVideoFrame/swapchain presentation, loop/rate control, battery pause and poster readback | MSVC compile-check only. Native icon layering, frame pacing, codec availability, mixed DPI, hotplug and fullscreen/occlusion pause are unverified; helper hotplug/occlusion support is not implemented. Daemon lock/sleep stops remain in place. |
| Linux X11 | One desktop/below/sticky X window per monitor; one dynamically loaded GStreamer decode pipeline, tee/crop/scale into monitor sinks; bounded queues, two decode threads, VA decoder preference when a render device exists, battery/coverage holds | Real Xvfb two-monitor seam, moving frames across loops, coverage pause/resume and parent-EOF exit passed. CPU colorspace/crop/scale/XImage presentation is **not** a zero-copy GPU path. Physical desktop/compositor and hardware-accelerated acceptance remain open. |
| GNOME Wayland | Unavailable in this path | Window explicitly refuses it. The existing Shell spike is not relabelled as a production extension. |
| KDE Wayland | Unavailable | No Plasma wallpaper plugin or D-Bus scripting integration. |
| wlroots / COSMIC / Hyprland | Unavailable | No layer-shell backend in this implementation. |

The Mac spanning window avoids pretending that attaching several AVPlayerLayers
to one AVPlayer is a proven multi-output renderer. A proper per-screen,
single-decode GPU fan-out remains required before Mac multi-Space sign-off.

Other requested work **not implemented**: HEVC import/transcode/faststart and
resolution/bitrate optimization, seamless replacement/crossfade, native still
wallpapers with no player, per-monitor stop of all covered outputs, actual
integrated-GPU device selection, and transport of true per-monitor scale
metadata (the shared math supports scale but current ctl conversion uses 1).

## Linux measurement record

`scripts/test-live-wallpaper.py` generates its own WebM using system GStreamer,
defines two 640×360 XRandR monitors in Xvfb, verifies the seam pixels and
continued motion after repeated loops, covers/un-covers the desktop, and
checks stdin-EOF teardown. It needs Xvfb, xrandr, system GStreamer with the
test-source/VP8/PNG/XImage plugins, and Python Pillow. It reads the helper from
`$CARGO_TARGET_DIR/debug/one-mk-wallpaper`; it installs nothing.

Measured here in a debug helper build with 30 fps VP8 software decode and two
640×360 destinations. CPU is user+system time; 100% means one logical core.
These are small synthetic clips, not representative hardware-decoder claims.

| State | Helper CPU | GPU |
| --- | ---: | --- |
| 1920×1080 loop, 10-second sample | 34.3% | Not measurable: no `/dev/dri` |
| 3840×2160 loop, 10-second sample | 115.8% | Not measurable: no `/dev/dri` |
| Both outputs covered, 5-second sample | 0.2% | Not measurable |
| Wallpaper off, daemon null backend, 2-second sample | 0 process CPU ticks | No helper |

**This does not meet the “cost almost nothing” requirement.** Covered playback
is paused but the helper's remaining monitoring is not literally zero CPU.
Lock/sleep child-reaping is tested, but there is no real Linux desktop-lock or
Mac hardware measurement. There is no frame-time/drop counter proving perfect
smoothness. The Xvfb tests prove crops, looping motion and lifecycle only.

Screenshots and raw measurements for this run are in `/tmp/wallpaper-shots/`:
`page.png`, `offline.png`, `compact.png`, `unavailable.png`, `x11-seam.png`,
`1080p-loop.png`, `1080p-motion.png`, `4k-loop.png`, `4k-motion.png`,
`measurements.json`, and `xrandr.txt`. Window screenshots use synthetic daemon
status, not a real paired Mac/PC; the X11 seam and motion screenshots are
actual system-decoder playback.

## Validation

Geometry tests cover multiple computers/monitors, mixed scale, negative
origins, offline reservation and all three fit modes. Clock tests cover offset
estimation, overflow, wraparound and correction thresholds. Cache/transfer tests
cover digest identity, tamper repair, retention, cache-hit byte skipping and
path rejection. Child tests cover off/shutdown, lock, sleep and last-owner
drop. The new off-idle integration test checks the actual daemon process.

Native cross-check scope is `one-mk-wallpaper`, `one-mk-app`, `one-mk-ctl` and
`one-mk-input` for `aarch64-apple-darwin`, `x86_64-apple-darwin` and
`x86_64-pc-windows-msvc`. The daemon is not Mac-cross-checked: its `ring`
dependency needs an Apple SDK/toolchain not present here. This is not a native
runtime or link/sign/notarization test. No fat-LTO build was performed.

Final verification record:

- The required `cargo clippy --workspace --all-targets -j 2 -- -D warnings`
  passed. A subsequent focused all-target Clippy run for the changed daemon,
  app, ctl, proto and wallpaper crates also passed.
- The required single `cargo test --workspace -j 2` run stopped on a legacy
  CLI-help expectation. That expectation and a later legacy renderer-install
  QA expectation were replaced to match the new CLI. The final
  `cargo test -p one-mk --tests -j 2` passed: 435 daemon unit tests, 22
  integration tests, and 6 ignored unit tests. The rest of the workspace
  passed via `cargo test --workspace --exclude one-mk -j 2`; focused shared
  geometry/ctl/proto tests passed again afterward. The initial full-workspace
  invocation itself was not a clean pass.
- All three native target checks above passed. The Mac helper additionally
  guards the macOS 12 Low Power Mode selector for the bundle's macOS 11 floor
  and uses a CFString value for the IOKit AC-power key, not a nonexistent
  exported C constant. These were rechecked for both Mac targets.
- Refreshed debug binaries passed `scripts/check-packaging.sh`: helper
  inclusion, Debian/AppDir/Flatpak contents, checksums and installer artwork.
  Ruby checks were skipped because Ruby is not installed. This does not
  verify native installers, code signing or actual Flatpak decoder access.
- `git diff --check` passed. No protected style/glass/chrome/widgets or input
  implementation files were changed, and `cargo fmt` was not run.

Build outputs used `/home/samer/.t3/cargo-target/wallpaper`, two Cargo jobs and
no local fat LTO. The build directory is removed at handoff; screenshots and
measurement evidence remain in `/tmp/wallpaper-shots/`.

## New dependency licences

Checked against package manifests / Cargo metadata and the base lockfile:

| New lockfile package | Version | Licence |
| --- | --- | --- |
| rfd | 0.15.4 | MIT |
| ashpd (rfd portal dependency) | 0.11.1 | MIT |
| async-fs | 2.2.0 | Apache-2.0 OR MIT |
| async-net | 2.0.0 | Apache-2.0 OR MIT |
| pollster | 0.4.0 | Apache-2.0 / MIT |
| urlencoding | 2.1.3 | MIT |
| windows-sys | 0.59.0 | MIT OR Apache-2.0 |
| objc2-av-foundation, objc2-core-audio, objc2-core-audio-types, objc2-core-media, objc2-core-video, objc2-metal | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| one-mk-wallpaper (our code) | workspace version | MIT OR Apache-2.0 |

New direct uses already present in the base lockfile: libloading 0.8.9 (ISC),
x11-dl 2.21.0 (MIT), image 0.25.10 (MIT OR Apache-2.0, PNG-only in the helper),
objc2 0.6.5 / the AppKit/Foundation/QuartzCore 0.3.2 bindings
(Zlib OR Apache-2.0 OR MIT), and windows/windows-core 0.62.2
(MIT OR Apache-2.0). AVFoundation, Media Foundation and Linux GStreamer are
system components; no FFmpeg, libmpv or x264 is bundled. System-installed
GStreamer plugins have their own licences. No Muro, Aura, Hanabi or Lively
source was copied.

## Owner's Mac acceptance checklist

1. Install the bundle with its wallpaper helper. In Wallpaper, choose H.264
   and HEVC clips using the native chooser, then by drag/drop. Check the poster
   seam and the three fit controls. Invalid media must produce a plain error.
2. With two monitors, test horizontal, vertical and negative-origin placement,
   including one Retina and one non-Retina display. Verify the real video seam
   and desktop-icon clicks. Test separate Spaces, Mission Control and a
   fullscreen app on each monitor: the spanning-window limitation is a blocker
   if either display is blank or attached to the wrong Space.
3. Watch at least ten loop boundaries. Check that sync rate nudges do not
   visibly stutter. Change videos while playing and record flashes; seamless
   replacement is not implemented yet.
4. Combine user pause, covering all displays, lock/unlock, screen sleep/wake,
   unplugging power and Low Power Mode. Removing one hold must not resume
   while another remains. Confirm off, lock and daemon quit leave no helper.
5. Measure 1080p and 4K in Activity Monitor for a minute each, including GPU
   history, then repeat covered/locked. Record hardware, codec, dimensions,
   fps, battery mode, CPU, GPU and dropped-frame observations. Do not accept
   “0% / smooth” based on compile checks or the Xvfb numbers.
6. Pair a second computer and align all monitors in Layout. Choose the clip
   once, confirm automatic copying, the shared seam and matching time. Test
   different source frame rates, congestion and pauses. Disconnect/reconnect
   the second computer: its canvas space must not move. Turn off while it is
   offline, reconnect it, and verify it stops. Repeat with a receiver that
   cannot decode the selected codec and record the currently missing ready
   acknowledgement/status behavior.
