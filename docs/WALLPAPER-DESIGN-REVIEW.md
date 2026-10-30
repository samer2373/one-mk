# Review of docs/WALLPAPER-DESIGN.md (independent, 2026-10-30)

The design document (branch `t3/research-live-wallpaper-design`) was reviewed by
a second model that re-read the sources it cites. **Verdict: not safe to build
from as written for the release-one scope and schedule; safe to start the
phase-1 spikes** (the geometry, one decode feeding N crops, and the GNOME
experiment), which is what wave 6 does first. The design must be revised with
the fixes below once the spikes report. Evidence labels: "read" = a page was
fetched, "local" = a read-only probe on the dev box, "judgment" = reasoning only.

## Fix first (before any implementation)

1. **Transcode and encoder story is inconsistent, and a Commons-first gallery
   makes it a core problem.** Commons video is almost all WebM (VP8/VP9) or Ogg
   Theora; Windows and macOS native stacks do not reliably play those, so
   nearly every gallery item needs a one-time transcode. The design calls H.264
   the "first common candidate", but an LGPL FFmpeg has no H.264/HEVC software
   encoder (libx264 and x265 are GPL; read: ffmpeg.org/legal.html); the
   installed GStreamer `x264` plugin reports `License GPL`, `openh264` BSD,
   `libav` LGPL (local). Decide up front: VP9/AV1 through BSD libvpx/libaom, or
   per-OS hardware/system encoders; add a per-OS decode table for that choice;
   forbid a GPL encoder in shipped artifacts and enforce it at build time by
   reading plugin licence metadata.
2. **The 4096x2160 decode cap contradicts the headline feature.** Three 1080p
   monitors are 5760 wide, three 4K are 11520: a 4096-wide source is upscaled
   1.4x to 2.8x. Add a "source resolution versus canvas width" section: tie the
   cap to the canvas, or define per-device sliced representations.
3. **The GNOME bridge licence.** The GNOME Shell review guidelines (read:
   gjs.guide/extensions/review-guidelines) require extensions on
   extensions.gnome.org to be GPL-2.0-or-later compatible and to contain no
   binaries, and discourage spawning privileged subprocesses. If we ship only
   with one-mk (not listed on extensions.gnome.org) those rules do not bind us;
   if we want the listing, the bridge must be GPL-2.0-or-later in its own
   directory, with the renderer shipped separately. Decide now.
4. **Hanabi precedent (read):** one GTK4 renderer window per monitor, found by
   application id; the extension adds a `Clutter.Clone` of each window actor into
   the Shell background actor; the window is kept minimized at the bottom of the
   stack; there is a workaround for a Mutter bug. So "external renderer cloned
   in the Shell" works on GNOME 45-50, but it is N windows, not "one texture, N
   clones", and Mutter bugs are version-specific: carry that as a maintenance
   cost. Frame callbacks for minimized windows are the real risk: a phase-1 stop
   gate. Do not copy Hanabi's GPL-3.0 code.
5. **Sync: presentation latency dominates the error.** Clock offset on a LAN is
   good to a few ms (NTP-style), but compositor/DWM/Mutter latency (1-3 frames),
   different scanout phases and Wi-Fi power-save wake delay (about 100 ms on the
   first packet after idle) dominate. Define two metrics (clock error and display
   error); add per-device presentation-latency calibration (auto plus a manual
   nudge slider); schedule by predicted present time; drop the first probe after
   idle; require filmed measurement for the 50 ms claim.
6. **Clock-sync plumbing.** The QUIC connection belongs to the daemon, so the
   daemon must stamp receive and transmit times; probes should be QUIC datagrams
   (no head-of-line blocking) kept off the bulk-transfer stream; say why
   GstNetClientClock/GstPtpClock are not used; specify the topology when A-B and
   B-C are paired but A-C are not.
7. **Rate nudging through the decoder is fragile** (GStreamer instant-rate-change
   has open regressions). Safer: the engine owns frame selection from the mapped
   clock (+-0.5% is one repeated or dropped frame about every 7 s at 30 fps).
   Normalised media needs short closed GOPs (1-2 s) for corrective seeks and late
   joins; most Commons clips are not loops, so seamless loops need a crossfade or
   ping-pong preparation step; define the epoch rebase when the rate changes.
8. **Hybrid GPU (local):** the dev box has an Intel TigerLake iGPU plus an NVIDIA
   RTX 3060 Mobile; GStreamer exposes Vulkan and nvcodec decoders on the NVIDIA
   GPU. Decoding there wakes the dGPU and ruins the power estimate. Add GPU
   selection (PRIME/Optimus, per-app GPU on Windows, automatic switching on
   macOS) to design and tests, with decode pinned to the integrated GPU by default.
9. **The 28-42 engineer-week estimate is not credible** (judgment: 70-110 weeks
   for three OS adapters, GNOME 45-50, a Plasma plugin, wlroots, sandboxing, a
   signed catalogue, a clock protocol, a multi-computer lab and a 24 h soak).
   Relabel it an optimistic floor or cut release one. (The owner already cut
   release-one Linux scope to GNOME only.)
10. **Plasma** carries the biggest hidden risk (decode inside plasmashell; DMA-BUF
    into the Qt scene graph is real C++). Not in release one by the owner's choice.

## Also fix

11. **Commons-only gallery** is practical only just: of 11,649 CC0 videos on
    Commons (API, 30 Oct 2026) 4,667 are wider than 1900 px and 519 wider than
    3800 px, but real loops are rare (the time-lapse category has 262 videos, 4 of
    them CC0). Expect about 20-100 curated items: call release one's gallery "tens
    of items". The CC0 tag is a per-uploader claim with no Foundation warranty;
    add periodic re-checks of Commons deletions and licence changes, and decide
    whether a signed index needs a DMCA/DSA contact and counter-notice process.
12. **URL import and peer media security gaps:** pin the validated IP (no
    re-resolve), block IPv4-mapped IPv6, odd IP encodings, unusual ports, proxy
    environment variables, the cloud metadata address, and HTTPS-to-HTTP redirect
    downgrades; bound container bombs (frame count versus duration, huge declared
    sizes, MKV attachments, extra streams); catalogue signing needs an offline
    root, rotation and freeze/rollback protection (TUF-style); a coordinator's
    transcode is not trusted by peers (every peer still decodes in the sandbox);
    the sandbox is a phase-1 gate, not a hope.
13. **Omissions:** pause or show a poster when the OS reduced-motion setting is on
    (SPI_GETCLIENTAREAANIMATION, accessibilityDisplayShouldReduceMotion, GNOME
    enable-animations) and warn about photosensitivity; pause in remote-desktop
    sessions (SM_REMOTESESSION); multi-user and fast user switching; closed lid with
    an external monitor; Windows fullscreen via SHQueryUserNotificationState; HDR
    (SDR white-level mismatch across monitors); screenshots and recordings; VRR;
    state that there is no telemetry (a gallery fetch discloses the IP to
    Wikimedia); disk-cache eviction (LRU, orphan cleanup, never the active scene).
14. **Windows WorkerW/Progman** is hedged correctly (24H2 broke wallpaper apps:
    read: The Register 2025-04-14): add a fail-closed check per Windows build and
    treat 25H2/26H1 as separate test cases; a single window spanning the virtual
    desktop avoids per-monitor swapchain sharing (N swapchains means N GPU copies).
15. **Licence packaging traps:** macOS hardened-runtime library validation
    conflicts with LGPL replaceability unless the disable-library-validation
    entitlement is used; an AppImage bundling the whole GStreamer tree pulls in
    `x264` (use an allow-list and a build-time check); a Debian system libavcodec
    may be GPL-configured; libmpv is GPL-2.0-or-later by default (LGPL possible;
    `-Dgpl=false` alone is no licence grant).
16. **Overconfident framing:** "feasible" holds only with far more than budgeted;
    treat hidden-window frame throttling as a phase-1 stop gate.
17. **macOS:** `AVPlayer.setRate(_:time:atHostTime:)` needs a prerolled item with
    `automaticallyWaitsToMinimizeStalling` off; WebM/VP9 support in AVFoundation is
    not established (ties to 1).
18. **Mixed physical DPI** across computers makes a "logical pixel" canvas look
    mismatched in the first release (physical calibration is later): say so.
19. **Power:** include the compositor's own redraw cost (Mutter/DWM repainting a
    video-backed background at 30-144 Hz); it cannot be paused by the engine; no
    measurement exists for a hybrid-GPU laptop.
20. **Bridge update and rollback** is mentioned, not designed: define a version
    handshake, a static fallback, and a "bridge older than engine" policy.

## Gallery terms: audit verdicts re-checked (all read 2026-10-30)

- Pexels not approved: correct (API terms forbid "making Pexels content available
  as a wallpaper app"; licence forbids redistribution on wallpaper platforms).
- Pixabay not approved: correct (terms, updated 20 Oct 2026, bar "Standalone"
  distribution, list wallpapers, and bar bulk or systematic copying).
- Mixkit not approved: correct (terms 9.4, 9.5, 9.6, 9.10).
- Coverr not approved: correct (cannot be offered by "mobile apps builders";
  bars compiling a similar or competing service).
- Wikimedia Commons: verdict and rules right (attribute the creator, hotlinking
  discouraged, jurisdiction-dependent public domain, no warranty; API needs an
  identifying User-Agent with contact and no white-label clients).
- NASA: conditional candidate is right (third-party material is marked; credit
  NASA; no implied endorsement). Internet Archive: terms unreadable: stays unverified.

## Claims verified correct

Host (GNOME Shell 50.1 on Wayland, one eDP-1 at 1920x1080 144 Hz, scale 1.25, so
1536x864 logical; GStreamer 1.28.2; `gtk4paintablesink` and `vah264dec` absent;
no ffmpeg or mpv on PATH), the layout and memory arithmetic, the NTP four-timestamp
formulas, mpv, FFmpeg and GStreamer licence statements, Hanabi's GNOME 45-50
support and GPL-3.0, and that the Progman 0x052C message is real but not a
supported Microsoft contract.
