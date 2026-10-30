# Live wallpaper: a continuous canvas across computers

Research/design only · 30 October 2026 · baseline `9643605` (`main`).

## Plain-language recommendation

Build **one-mk-wall**, an optional program that plays a moving background,
separate from the keyboard/mouse daemon. Treat all participating screens as
windows looking into one large picture. Each computer downloads the same
approved media, decodes it locally once, and shows its assigned rectangles at
the agreed time. The first public release must support this across computers,
not merely across the monitors plugged into one computer.

Support Linux, Windows and macOS; support video, GIF, APNG and animated WebP.
Use native Windows/macOS media facilities and GStreamer on Linux, behind a
small common playback contract. GNOME needs an explicitly installed Shell
integration; KDE needs a Plasma wallpaper integration. Neither is solved by
opening a normal fullscreen window. Keep networking, downloads, decoding and
animation timers out of the KVM daemon when wallpaper is off.

**This is feasible, but not a frame-locked video wall.** Adjacent monitors on
different computers may show slightly different moments, especially on Wi-Fi.
Do not promise invisible seams. A local decode/clock prototype and real OS
tests are release gates, not polish to defer until after shipping.

| Decision | Recommendation | Consequence |
|---|---|---|
| Delivery | Three internal milestones; multi-device in release one | Milestone 1 alone is not a shippable answer |
| Engine | Native per-OS adapters, one decode per computer, N outputs | More adapter work, fewer shipped media dependencies on Windows/macOS |
| GNOME 50 | Optional, independently authored Shell bridge | Privileged integration; must pass disposable-session tests first |
| Plasma | Optional wallpaper plugin, not layer-shell | Plugin/runtime licensing and Qt texture integration need a gate |
| Canvas | Shared logical-pixel layout; include gaps by default | Optional gap compression and later physical calibration |
| Synchronization | Monotonic epoch, measured clock offset, gentle rate correction | Best effort; measured quality indicator, no frame-exact claim |
| Content | Own files, bounded HTTPS import, curated licensed download gallery | No videos bundled in source or installers |
| Gallery | Start with audited Commons CC0/public-domain items | Pexels/Pixabay/Mixkit/Coverr not approved feeds |
| Idle | Wallpaper engine absent when disabled; zero new periodic daemon work | Playing video necessarily consumes resources |
| Scope | No web view, shaders, lock-screen replacement or automatic theming in release one | Shaders only a possible later extra |

### Evidence and uncertainty key

**VERIFIED-REPO** means inspected in this checkout. **VERIFIED-HOST** means a
read-only probe succeeded here. **DOCUMENTED [Sxx]** means supported by the
linked primary documentation, not exercised here. **PROPOSAL** means a design
choice, budget or acceptance threshold, not an existing capability.
**UNVERIFIED** explicitly marks missing evidence, uncertain integration, legal
clearance or unmeasured performance. All future behavior below is PROPOSAL
unless marked otherwise; a proposed API name is not a claim it exists.
Sources were accessed on 30 October 2026. Recheck moving branches, contracts,
SDK availability and the final dependency lockfile before implementation.
This is an engineering licence assessment, not legal advice or legal approval.

## 1. What was actually inspected

### Repository contracts (VERIFIED-REPO)

- `Cargo.toml`: workspace licence `MIT OR Apache-2.0`, version 0.1.13.
  `README.md` and `docs/FEATURE-PLAN.md`: low latency, zero idle CPU, small
  footprint, native UI/no web view, optional app/tray. Windows/macOS input
  backends are described as compile/unit-tested, not real-hardware validated.
- `crates/proto/src/geometry.rs`: `Rect` is signed integer logical pixels;
  `Desktop` has monitor rectangles, bounds, gap-aware clamping and touching
  edges. `motion.rs` applies placement offsets to pointer crossings. Reuse
  rectangles/transforms, **not** pointer clamping for wallpaper texture lookup:
  clamping would smear the picture across gaps.
- `crates/proto/src/codec.rs`: `Control::Layout` carries screens and a trailing
  optional offset; capability flags and `MAX_SCREENS` already bound protocol
  data. `PROTOCOL_VERSION` is 2. New messages require explicit negotiation;
  do not assume old peers ignore unknown tags.
- `crates/ctl/src/status.rs`: `Status.screens`, `offsets`, boxed `layout` and
  peer monitor data provide the Layout tab's evidence. Placement is relative,
  not already a globally agreed, versioned scene graph. Side-only placement
  does not identify a unique continuous canvas.
- `crates/proto/src/transfer.rs`: `Offer`, `Accept { offset }`, `End { hash }`,
  `Saved`/`Failed`; final hash is BLAKE3. Resume uses offered size/mtime.
  `crates/one-mk/src/transfer.rs` handles file transfers. Wallpaper must reuse
  the transport/integrity mechanisms without treating arbitrary Downloads
  paths as playable media or weakening transfer authorization.
- `crates/one-mk/src/control.rs`, `settings.rs`, `crates/ctl`, `crates/app`,
  `crates/tray`, `docs/PACKAGING.md`, `.github/workflows/release.yml` and
  `scripts/{bundle-deb,package-appimage,bundle-mac}.sh` are integration points.
  No product files were changed for this research.

### Host capability record (VERIFIED-HOST, not a playback test)

Read-only commands: `gnome-shell --version`, `printenv`, `command -v`,
`pkg-config --modversion`, `gst-inspect-1.0`, `gsettings get`, `busctl --user
list`, D-Bus `Introspect` and `DisplayConfig.GetCurrentState`. Read installed
`gdbus(1)`, `gst-inspect-1.0(1)` and `Xvfb(1)` man pages. The first sandboxed
bus probe was denied; an approved read-only retry succeeded. Introspection
lists mutating methods too: **none of those methods was called**.

| Probe | Observation | What it does NOT establish |
|---|---|---|
| Session | `wayland`, `ubuntu:GNOME`, `wayland-0`; GNOME Shell **50.1** | No older GNOME/X11 or other compositor exercised |
| Monitors | One `eDP-1`, current 1920×1080 at about 144.005 Hz, scale 1.25, position (0,0), transform 0 | No physical multi-monitor test; logical extent inferred as 1536×864 |
| Shell extensions | Enabled list includes DING, Ubuntu Dock, tiling assistant and two search providers; global disable flag false | No Hanabi/one-mk wallpaper bridge found in inspected extension directories; list is not proof of every possible integration |
| Display D-Bus | `MonitorsChanged`; `PowerSaveMode` property | Not a public wallpaper/video API |
| ScreenSaver D-Bus | `ActiveChanged(bool)` | No authorization to draw on lock screen |
| Power | UPower 1.91.1 exposes `OnBattery`; PowerProfiles 0.30 exposes `ActiveProfile` | No battery transition tested; do not set a profile |
| Media runtime | GStreamer 1.28.2; `playbin3`, `appsink`, `glimagesink`, `avdec_h264` found | Decoder existence is not successful hardware decode |
| Selected missing elements | `gtk4paintablesink`, `vah264dec`, `vaapih264dec` not found | Other hardware paths may exist; GPU/driver support **UNVERIFIED** |
| Development metadata | No version returned for GStreamer/video/GL, libmpv, libavcodec, EGL, wayland-client or GTK4 `.pc` checks | Runtime libraries may exist without development packages |
| Executables | `Xvfb`, `gdbus`, `busctl`, `gst-inspect-1.0`, `pkg-config` found; mpv, mpvpaper, ffmpeg, wayland-info, weston-info, Sway, Weston not found on PATH | Headless Wayland and transcode experiments cannot be claimed complete |

No packages were installed; no extension, compositor, input backend, player or
test daemon was started. No wallpaper, desktop setting, power setting or live
window was changed. No compile, playback, power measurement or E2E test ran.
No throwaway probe files were needed. Host observations do not certify a
shippable GNOME backend.

## 2. Inspiration without copying

Muro's public README describes native looping video, gallery downloads,
playlists/time schedules, menu-bar controls, one-time HEVC imports, automatic
pauses and a 30 fps mode; its macOS 26 lock-screen/saver offering is distinct
from ordinary desktop playback [S01]. Aura's README describes Rust/Linux
integration, mpv/mpvpaper, scaling/monitor controls, catalogues, playlists,
switching HUD, pause rules and colour theming [S02]. Its current README also
advertises GNOME and Plasma integrations, so it is not accurate to describe
today's Aura as COSMIC-only. These are feature descriptions, not validated
performance claims or architecture specifications.

**Hard boundary:** use neither project's implementation, translated code,
close paraphrases, assets nor catalogue. Muro is source-available under
PolyForm Shield 1.0.0, not unrestricted open source; treat it as prohibited for
this implementation. Do not rely on the brief's pre-21-September-2026/v5.0 MIT
history (**UNVERIFIED**, deliberately not investigated). Aura is GPL-3.0:
do not copy or link its code into one-mk. No implementation files from either
were read. Hanabi is likewise only a README-level example of a GNOME Shell
extension plus external video renderer [S03]; do not copy its GPL code.

## 3. Getting underneath desktop icons

### Windows: Explorer-hosted wallpaper surfaces

DOCUMENTED: `SetParent` can reparent a window, but cross-DPI-awareness
reparenting can reset awareness or fail. DirectComposition can compose GPU
surfaces; device loss requires recovery [S04, S05].

The familiar wallpaper technique discovers `Progman`, `SHELLDLL_DefView`
(desktop icon view) and `WorkerW`, sometimes asks Explorer to create a worker
using undocumented message `0x052C`, then parents a nonactivating, input-passive
window beneath the icons. **UNVERIFIED integration here:** the hierarchy and
message are not a supported Microsoft wallpaper contract. Windows 11 24H2
changed Explorer behavior; older sibling-WorkerW assumptions are unsafe.
Microsoft documents wallpaper-app compatibility failures on 24H2 [S06].
That evidence does not establish one universal replacement hierarchy. Detect
actual parent/child ordering; test both raised-desktop/child-WorkerW and older
arrangements, Explorer restart, virtual-desktop switch and shell updates.
Fail closed to the existing static desktop if no safe host is found; never
hide/kill Explorer, send endless recreate messages or cover icons as fallback.

Adapter proposal: Media Foundation source reader / hardware-capable transform
decodes once into D3D11 textures; DirectComposition/DXGI output per monitor
samples the common texture. AV timing must be externally steerable; a separate
player per output is not the production design. A libmpv render/D3D route is
an alternative only after the build-specific licence gate in section 9.
Media Foundation and hardware acceleration facilities exist [S07]; actual
codec/profile/size/adapter compatibility is runtime-probed, not assumed.

Use per-monitor DPI-awareness v2 deliberately. Enumerate monitor physical
rectangles and effective DPI, convert into one-mk logical coordinates once,
and keep an inverse map for presentation. `SM_XVIRTUALSCREEN`/Y can be negative;
do not use primary-screen dimensions as the desktop. Handle `WM_DISPLAYCHANGE`,
`WM_DPICHANGED`, rotation and GPU removal. Cross-GPU texture transfer may copy
or fail (**UNVERIFIED**); cap costs or use a clearly reported second decoder
only as an explicit fallback, not silently.

### macOS: desktop-level AppKit windows and native decode

DOCUMENTED building blocks: borderless `NSWindow`, appropriate window levels,
`canJoinAllSpaces`/`stationary`, `NSScreen`, AVFoundation/VideoToolbox [S08–S11].
Proposal: one non-key, nonactivating, mouse-ignoring window per `NSScreen`, at
desktop level **below** Finder's desktop-icon level, not desktop-icon level
itself. Recreate/reposition on screen-parameter changes; test Mission Control,
Spaces, Show Desktop, Stage Manager and fullscreen Spaces. Collection behaviors
alone do not prove all these combinations work (**UNVERIFIED on real macOS**).
AppKit points are logical units; backing scale determines surface resolution.
Convert AppKit's axis/origin conventions explicitly.

`AVPlayerLayer` is the simplest native video presentation candidate. Do not
claim N layers or N players guarantee one decoder or perfect synchronization.
For the one-decode contract, own a single decode/output path, share each
decoded `CVPixelBuffer`/IOSurface, and render crops through native layers or
Metal. `AVSampleBufferDisplayLayer.controlTimebase` controls display timing;
multiple layers can be driven from a common host-clock timebase. Apple's
`AVPlayer.setRate(_:time:atHostTime:)` offers a scheduled-start building block
[S09, S10]. Queue-depth/backpressure and sharing behavior need a prototype.
Probe VideoToolbox support and require successful hardware-session creation,
not merely a codec name [S11].

**Correct an overbroad premise:** third-party screen savers existed before
macOS 26; Apple documents `ScreenSaverView` [S12]. That is not permission to
replace the secure login/lock-screen wallpaper with an ordinary app window.
Muro advertises macOS-26-specific lock-screen/saver integration [S01], at idea
level selecting media for OS-managed playback. Its underlying mechanism,
public-API stability and suitability for one-mk are **UNVERIFIED** from its
README. Do not infer Apple has a generally supported third-party live-lock-
wallpaper API, before or after 26. Exclude this integration from release one;
never bypass lock security, modify protected databases or rely on private APIs.

### Linux X11: the root window is often not the visible desktop

EWMH defines `_NET_WM_WINDOW_TYPE_DESKTOP` [S13]; it is a stacking hint, not a
portable promise to sit between another desktop application's background and
its icons. Updating a root pixmap is primarily a static-background convention,
not a good video transport. Repeated CPU uploads to the root are rejected.

| Desktop | Who owns the visible background/icons | Consequence for this design |
|---|---|---|
| GNOME on older X11 sessions | Shell background; optional desktop-icons extension or older file-manager desktop | Root video can be hidden. Extension-specific integration needed; GNOME 50 removed its X11 session [S14] |
| KDE Plasma X11 | plasmashell desktop containment draws wallpaper and icons | Use Plasma wallpaper plugin; root/desktop-type window is not sufficient [S15] |
| Xfce | xfdesktop manages background and icons [S16] | Desktop-type video may cover icons or sit below an opaque desktop; cooperative integration required |
| Cinnamon | Nemo handles desktop icons [S17]; Cinnamon also participates in background handling | Precise stacking/transparency varies: **UNVERIFIED**; test actual session, no unsupported promise |
| MATE | Caja manages desktop/background [S18] | Root can be covered; exact version/configuration stacking **UNVERIFIED** |
| Bare X11 WM without a desktop manager | No competing opaque desktop | EWMH desktop surface plus GL rendering is the most tractable target; still test focus/stacking |

Do not disable any existing desktop/icon manager automatically. Xvfb proves
geometry and basic X rendering, **not** compatibility with these desktops.
For Xfce/Cinnamon/MATE with incompatible background ownership, advertise
unsupported until a cooperative backend passes the icon-preservation tests.

### Linux Wayland: three materially different integration families

1. **wlroots/Sway/Hyprland and COSMIC:** capability-detect
   `zwlr_layer_shell_v1`; create output-bound surfaces in the `background`
   layer, no keyboard interactivity, empty input region, no reserved work
   area. Follow configure/ack/commit lifecycle, output scale/transform and
   removal [S19]. A protocol in the compositor does not ensure its own
   background client will not cover ours: test/coordinate that separately.
   COSMIC/Hyprland/Sway support is documented by Aura/mpvpaper READMEs at idea
   level [S02, S20], **UNVERIFIED here**. Do not copy their implementations.
2. **KDE Plasma Wayland:** the Plasma desktop is above background layer-shell
   surfaces. KWin supporting layer-shell does not fix an opaque plasmashell
   desktop. Use an independently authored wallpaper plugin hosted in the
   desktop containment [S15]. The scripting API can select/configure a
   wallpaper plugin; it does not turn a video file into a renderer. Avoid
   decoding inside plasmashell: plugin displays engine-owned frames through
   a small native bridge. Texture import/private IPC and Qt module licence
   compatibility are **UNVERIFIED gates**; a crash must not take down Plasma.
3. **GNOME/Mutter:** no general wlr-layer-shell wallpaper route. An ordinary
   client or XWayland root trick cannot reliably place video between Shell's
   background and desktop icons. A custom Clutter actor in a Shell extension,
   or a Shell bridge presenting/cloning external renderer surfaces, is the
   credible route; Hanabi documents the latter product pattern [S03]. No
   remote-desktop/input permission is needed merely to draw the wallpaper.

### Exactly what the owner's GNOME 50 box can support

VERIFIED-HOST: GNOME 50.1 Wayland, DING enabled, no discovered live-wallpaper
bridge; external ordinary clients have no verified wallpaper surface here.
Neither the display-config D-Bus object nor background picture settings accepts
a live decoded frame stream. Installing mpvpaper alone would not solve this.

PROPOSAL: a deliberately thin, independently written extension identifies only
our authenticated local renderer's surfaces, suppresses their normal window
decorations/focus/task-switcher presence, and presents cropped actors at the
background position while preserving DING above them. The renderer owns one
decode pipeline, clock and frame distribution; Shell owns only presentation
and window-coverage observations. Multiple clones of a single texture are an
optimization to prove, not a promise. Hidden-window frame throttling,
Wayland actor cloning, maximum texture dimensions and efficient GPU sharing
with GNOME 50 are **UNVERIFIED**. Do not place a media parser/decoder in Shell
as an easier fallback. If external presentation fails the spike, stop and
report a GNOME release blocker rather than shipping an always-on-top window.

The extension could observe compositor windows/fullscreen/coverage, workspaces,
monitor changes and Shell session state. It cannot add a Wayland global to
Mutter, provide frame-lock across computers, safely change the secure login
screen, or give an untrusted peer arbitrary Shell execution. It runs with
Shell's privileges and can crash the session: keep its attack surface tiny,
accept no media URLs/code, bound all messages and disconnect everything on
disable. Exact layer placement relative to DING, Overview and lock screen is
**UNVERIFIED until isolated-session tests**, not demonstrated by introspection.

Ship an optional versioned extension ZIP/package with its own manifest/UUID
and a narrowly versioned bridge protocol. Install/enable only after an explicit
owner action, through normal GNOME extension tooling; per-user location is
`~/.local/share/gnome-shell/extensions/<uuid>`, system packages use
`/usr/share/gnome-shell/extensions`. Do not install it in this research task.
On Wayland explain when logout/login is necessary; do not restart the live
Shell. Test GNOME 45–50 separately: 45 changed extensions to ES modules [S21],
and 50 removed X11 support but its guide does **not** claim all extension APIs
changed [S14]. Pin tested versions, do not disable Shell version validation;
unsupported versions get a static fallback and an actionable doctor message.
Update signed one-mk/bridge artifacts together with rollback; an Extensions
website listing, if pursued, requires its separate review/update process.
Shell integration licensing must be reviewed independently (section 9).

## 4. One canvas: coordinates, gaps and one decoder

### Layout mathematics (PROPOSAL)

Use signed 64-bit intermediates and finite bounded transforms. For monitor
rectangle `R(d,m)` in device-local logical pixels and device placement `T(d)`,
global rectangle is `G(d,m) = T(d) + R(d,m)`. The visible canvas is the union
of those rectangles; its enclosing bounding box `B` defines media scaling.
Retain the holes in the union. Normalize by `B.min`, not by the primary monitor
or each device's bounds. Reuse existing rectangles but not unchecked `i32`
addition for a network-defined large canvas.

Example: A's monitor is `(0,0,1920,1080)`. B is placed at `(2020,-120)` and its
monitor is `(0,0,1280,1024)`. The box is `(0,-120,3300,1200)`, with a 100-pixel
horizontal gap. Monitor A samples global y starting at normalized y=120;
B samples x=2020, y=0. Never independently fit the same movie to A and B.

- **Include gaps (default B):** keep unlit pixels in the canvas, so an object
  disappears behind the 100-pixel space before reappearing. Layout units are
  not physical millimetres; visually physical continuity still needs manual
  bezel/gap calibration if sizes/scales differ.
- **Compress gaps (option A):** explicit virtual packing maps monitor
  rectangles to a gapless logical arrangement. A simple horizontal strip can
  subtract gaps; arbitrary L-shaped/overlapping arrangements have no unique
  continuous compression. Require an editable deterministic packing preview,
  never an undocumented nearest-monitor warp. Some content lines cannot stay
  continuous at every edge after such packing. Defer arbitrary packing until
  the included-gap mode passes; label this option's limitations.
- **Optional physical calibration later:** EDID millimetres plus orientation
  can map every monitor into common millimetres; provide manual width/height
  and bezel corrections, because EDID may be absent/wrong. Physical size is
  not present in the inspected `Rect` protocol; negotiate new metadata, do not
  silently reinterpret KVM movement coordinates.

For source dimensions `(Vw,Vh)` and canvas `(Bw,Bh)`, Fit uses uniform factor
`min(Bw/Vw,Bh/Vh)` plus letterbox; Fill uses `max(...)` plus crop; Stretch uses
separate factors; Centre uses 1 source pixel per canvas logical pixel and clips
or pads. Anchor defaults to centre. Apply this transform **once per canvas**;
for each output invert it on that output's global rectangle to get UV/source
crop coordinates. Clamp sampling only to media bounds, fill padding with a
chosen colour, and use pixel-centre conventions consistently to avoid seams.
Bake source sample-aspect-ratio/rotation into display dimensions first.

### Decode and presentation

One computer decodes one selected representation once, then N surfaces sample
the same immutable frame and timestamp. Do not allocate a giant RGBA canvas
whose dimensions equal the complete cross-computer wall. Decode a bounded
source frame and render viewports. Different computers necessarily have their
own decoders; "one decoder" is per device/media, not one network-wide decoder.

Use a small bounded frame queue and explicit GPU ownership/fences. Linux:
GStreamer decoder → GL/DMABUF-compatible frame distribution → output surfaces;
CPU `appsink` upload is a correctness fallback, not an efficiency claim.
Windows: shared D3D textures. macOS: shared pixel buffers/Metal or sample-buffer
layers. These zero-copy paths, especially across GPU adapters and Shell/Plasma
process boundaries, are **UNVERIFIED** until measured. Common trait contracts:
prepare, scheduled start, target timestamp/rate, crop updates, pause, frame
position report, drain/stop, hardware-decoder status and error reporting.

N decoders multiply decode surfaces, initialization and memory bandwidth and
can drift independently. They may be acceptable as an opt-in compatibility
fallback, never the default or the phase-1 acceptance path. A slow output
drops/repeats frames; it never blocks other outputs or the KVM.

Refresh rates are independent: a 30 fps frame can repeat at 60/144 Hz; there is
no common physical scanout epoch. Use compositor/frame callbacks and PTS,
not a 144 Hz busy loop. On mixed 60/144 Hz screens the displayed frame can
differ by a refresh even on one GPU. Fractional HiDPI affects buffer sizes,
not global crop geometry. Portrait displays apply native output rotation
after logical crop; mirrored outputs must not expand the union twice.
Limit release one to SDR sRGB/Rec.709; HDR/color-management parity across
screens is **UNVERIFIED**, not a promised seamless result.

Hotplug/mode/scale/rotation events produce a new layout revision. Keep the
playback timeline running, allocate new surfaces first, then atomically switch
crop transforms at a future agreed scene time; failed new outputs stay static.
Removing a device should normally reserve its rectangle rather than zoom every
other screen; offer explicit "reflow to online screens". The owner chooses
whether local hotplug also reserves missing space. Never persist a half-applied
layout or change KVM offsets implicitly.

## 5. Across devices: distribution, authority and clock sync

### A scene is more than a pointer placement

Choose an explicit coordinator from the owner's paired devices; all members
locally opt into wallpaper control by that coordinator. Pairing proves peer
identity, not ownership or unlimited consent. Freeze a session manifest:
session ID, coordinator fingerprint, generation, layout revision/hash,
participating fingerprints, global rectangles, crop/scaling mode, media ID,
representation IDs, duration/timebase, loop policy and future start epoch.

Resolve relative offsets into coordinator coordinates; validate cycles by
summing offsets, reject inconsistent cycles, ambiguous side-only placement,
duplicate IDs, unreasonable counts/extents and unintended overlap. Provide a
preview and placement action rather than guessing what "on your right" means.
KVM pairwise layout updates can race; do not make every peer independently
derive a different wallpaper graph. A frozen scene revision is the authority.
Only coordinator-approved revisions can change it. Retain offline rectangles.

### Media distribution

Use the existing paired QUIC connection and resumable, hashed file-transfer
machinery with a **new authorized wallpaper-cache purpose**, not a request to
play `/some/path`. Audit existing transfer scheduling and add strict lower
priority/bandwidth/concurrency bounds so large video transfers cannot starve
input/clipboard. A separate stream is not, by itself, a congestion guarantee.

1. Owner selects a file/URL/catalogue item locally. A restricted import process
   downloads/probes/normalizes it; peers never fetch a URL sent by another peer.
2. Hash original bytes with BLAKE3; record format, dimensions, frame times,
   duration, loop count, colour profile and bounded provenance/rights metadata.
   Validate before allocation. Use content hashes as cache names, not paths.
3. Negotiate actual decoder capability, including codec/profile/level, bit
   depth, max dimensions, hardware status and representation policy. A codec
   listed by an API is not proof of efficient playback. Default muted playback;
   strip audio and embedded attachments in normalized representations.
4. Transcode **once per required representation**, then distribute/cache those
   exact bytes; not once per screen or every loop. Prefer a common representation
   if quality/size allow. No universal HEVC mandate. Conversion is optional and
   explicitly consented; preserve the original and report conversion progress.
5. Only verified complete files become playable. Resume partial transfers but
   rehash the entire completed representation. Hash media identity and timeline
   identity separately: different encodings can represent the same content.
6. Each ready member pre-rolls its decoder. Start ready members at a future
   coordinator epoch; never wait indefinitely for one slow/offline computer.
   Late members join at current scene PTS after pre-roll, not at frame zero.

### Codec decision matrix (not a hardware guarantee)

DOCUMENTED native facilities: Media Foundation formats [S07], VideoToolbox
hardware-support query [S11], GStreamer plugin discovery [S22]. All specific
hardware/profile/OS combinations below are **UNVERIFIED on this host and must
be negotiated by a real decode test**. This matrix describes selection policy,
not a claim that every machine has the codec installed.

| Codec | Windows native | macOS native | Linux GStreamer | Release policy |
|---|---|---|---|---|
| H.264, 8-bit 4:2:0 | Standard native decoder; Media Feature Pack/edition exceptions | AVFoundation/VideoToolbox candidate on supported systems | Plugin/driver dependent; software decoder found here | First common MP4 candidate; probe level/resolution and hardware |
| HEVC | May require codec extension/licensing and compatible GPU | Strong modern-Mac candidate; Intel/old-model variation | VA/NV/V4L2/other plugin/driver variation | Optional efficient representation, never automatic paid-codec dependency |
| VP9 | Extension/OS/GPU dependent | Native container/profile and hardware path not assumed | Common plugin possibilities; installed HW **UNVERIFIED** | Enable only after per-device probe |
| AV1 | Extension + recent hardware often required for efficient use | Hardware only on some newer chips; exact model matrix **UNVERIFIED** | Recent GPU/driver/plugin needed for hardware | Optional; do not software-decode 4K silently |

MP4, MKV and WebM are containers, not codecs. Accept them as import sources
only when their actual contents pass the whitelist. Normalize to a tested
SDR representation when native backends disagree. Preserve common rational
frame timestamps, loop duration and frame count across transcodes; if needed
normalize variable-frame-rate sources once. Offline transcoding can use native
encoders or an audited LGPL FFmpeg build; availability of a suitable encoder
on every platform is **UNVERIFIED**. Codec patents/HEVC commercial terms are
separate from open-source copyright licences and need distribution review.

### Clock synchronization: explicit algorithm

Use monotonic clocks, not wall-clock UTC or the operating system's NTP setting.
Do not change system time. QUIC RTT estimates alone cannot determine clock
offset. Exchange four application timestamps over the authenticated control
channel: A sends at `t1`; B receives at `t2`, replies at `t3`; A receives at
`t4`. Estimate B−A offset `theta = ((t2-t1)+(t3-t4))/2`, network roundtrip
`delta = (t4-t1)-(t3-t2)`, following the NTP measurement model [S23]. Include
processing times, sequence/session IDs and monotonic-clock generation.

Take a bounded initial burst (proposal: 8 samples), filter invalid/outlier and
high-delay samples, use the lowest-delay sample set and a robust offset/drift
fit. QUIC's smoothed RTT informs sample timing/uncertainty only. Bound allowed
drift and offset changes; a peer cannot move another computer's OS clock.
Maintain a mapping from each local monotonic time into coordinator time;
invalidate it across sleep/reboot. Track uncertainty and sample age.

Proposal: `PTS(now) = (coordinator_now - epoch + media_start) mod duration`.
Start about 2 seconds ahead after readiness, adapt lead time to slow pre-roll.
Re-estimate every 5–15 seconds **only while wallpaper is active**, with a
bounded backoff under congestion. Schedule in the wallpaper process; daemon
only routes active-session messages. Fully disabled/all-paused sessions have
no wallpaper heartbeat. On resume do a fresh offset burst.

Compare presented PTS (not decoder dequeue time) to target. Inside a deadband
do nothing; for small errors use bounded rate nudges (initial tuning proposal
±0.5%, up to ±1% under tested policy), then return to 1.0. A 20 ms error takes
about 4 seconds to recover at 0.5%, so do not pretend this is instant. Large
errors/sleep use one deliberate pre-rolled seek, preferably at a loop boundary;
never seek every frame. Use seamless-loop preparation so loop correction does
not repeatedly flash black. If an adapter cannot rate-adjust images, it selects
the frame from the shared time schedule rather than running an independent
animation timer. Paused local outputs resume to current group PTS; an explicit
group pause instead freezes the shared epoch and increments generation.

**Numbers we can justify, and numbers we cannot:** path asymmetry creates an
offset error of half the forward/reverse delay difference. If that difference
is 10 ms, offset error is 5 ms before decoder/display delays. At 60 Hz scanout
quantization is 16.67 ms; at 30 fps content frames are 33.33 ms apart. A 50 ppm
clock drifts 0.5 ms in 10 seconds (illustrative oscillator assumption, not a
measured device). These are arithmetic, not Wi-Fi benchmarks. No numeric
"typical Wi-Fi accuracy" was verified here.

PROPOSAL quality targets: warm loopback pipeline P95 inter-process presented-
PTS error ≤20 ms; qualified real wired/Wi-Fi configurations P95 ≤50 ms,
P99 ≤100 ms after warmup, otherwise show "sync degraded". These are release
targets, **UNVERIFIED achievable**, not guarantees on arbitrary Wi-Fi. Measure
and publish RTT/asymmetry limitations and display error independently. At
300 pixels/second, 20/50/100 ms mismatch is a 6/15/30-pixel jump at a seam.
Fast pans make this visible. Content with slow movement tolerates it better.
Frame-exact synchronization needs display/genlock support beyond this design.

### Failure policy

| Failure | Local response | Other devices |
|---|---|---|
| Sleeps/locks/display off | Stop decode/present; retain bounded scene metadata | Keep playing; reserve missing rectangle |
| Rejoins late | Fresh clock sync, cache verification and pre-roll to current PTS | Never restart everyone |
| Disconnection | Free-run cached local slice, show status when UI opens | No barrier/wait |
| Coordinator disappears | Freeze last committed manifest and free-run; explicit owner takeover generates new authority epoch | Avoid competing automatic leaders in release one |
| Slow decoder | Drop frames, request smaller approved representation, or freeze a poster with reason | Never slow the group timeline |
| Corrupt/missing media or disk full | Reject, preserve previous working wallpaper, bound retries | Ready peers continue |
| Layout conflict | Retain last committed scene; explain exact conflict | No guessed rearrangement |
| Engine/bridge crash | Tear down our surfaces, native desktop remains; bounded restart backoff | KVM remains responsive |
| Forgotten peer | Revoke wallpaper authority immediately; end streams and discard uncommitted offers | Existing local authorized cached playback need not be remotely erased |

## 6. Security is part of the media architecture

An authenticated video can still be malicious. A paired-but-compromised laptop
is a realistic threat. FFmpeg publishes decoder security fixes [S24]; mpv's
history includes URL/script-hook exposure, not just pixel decoding [S25].
Do not claim that a hash, safe Rust wrapper or hardware decoder makes a video
safe. Hardware decode still uses parsers, drivers and firmware.

PROPOSAL default admission caps (tune only after tests): 512 MiB/source file,
2 GiB aggregate cache, 4096×2160 decoded pixels, 60 fps source maximum,
30 fps efficient representation, 10-minute loop, 2 queued imports, 1 concurrent
transcode, 16 MiB decoded/control metadata ceiling reduced to much smaller
per-message schemas, and 256 MiB decoded-animation working-set ceiling.
Reuse or tighten existing monitor caps; bound canvas extents, sample queues,
hashing buffers, network messages, retries and every integer multiplication.
Higher 8K/wide-panorama support is opt-in only after resource/codec tests;
large canvas does not imply large decode allocation.

- Authorize per coordinator and per purpose. Remote commands reference only
  validated cached media IDs and committed manifests. Never accept remote file
  paths, arbitrary commands, playlist URLs, shell fragments or local-file URIs.
- Import only explicit owner-requested HTTPS resources. Validate each redirect,
  disallow credentials and local/private/link-local destinations by default,
  defend against DNS rebinding, cap time/bytes/decompression, verify magic bytes,
  and sanitize names. A generic web-page downloader/yt-dlp integration is not
  in release one. URL imports are direct media, not DRM/paywall bypasses.
- Use a purpose-restricted parser whitelist: tested MP4/MKV/WebM plus
  GIF/PNG/WebP variants; no scripts, external references, embedded executables,
  network playlists, subtitle fetches or arbitrary demuxer protocols. Extension
  checks alone are insufficient. URL downloader exits before playback.
- Decode/probe/transcode in low-privilege processes with no pairing private
  keys, owner document access or network. Narrow file handles and bounded IPC;
  Linux namespace/seccomp where deployable, Windows restricted token/job or
  AppContainer feasibility spike, macOS sandbox/helper entitlement spike.
  Actual sandbox profiles and GPU access compatibility are **UNVERIFIED**.
- Prefer hardware decode for power, and offer a hardware-required policy that
  refuses unsupported video rather than silently burning CPU. This is **not a
  security sandbox**; bounded software image decoding is still necessary for
  GIF/APNG/WebP. Keep both patched, and allow low-resolution software video only
  after explicit consent. No decoder inside the daemon, Shell or plasmashell.
- The main engine may need display access; split an even narrower decoder
  child if robust GPU-handle transport is possible. Do not advertise isolation
  against other same-user processes as solved by an ordinary child process.
- Signed update/catalogue metadata, pinned schema versions, provenance records,
  revocation handling, crash redaction and dependency advisories/SBOM are release
  requirements. A checksum authenticates bytes only if its manifest is trusted.

## 7. Power, idle CPU and event-driven pause

**Mandatory invariant:** when disabled, `one-mk-wall` and its decoder/import
children do not run, media libraries are not linked/loaded into the daemon,
and no wallpaper timer/network polling remains. Starting the daemon alone
must never start a wallpaper. Optional autostart is explicit and separate.

The active engine maintains a set of pause reasons: manual, gaming, fullscreen,
covered, session locked, display asleep, battery, low battery and low power.
Resume only when all applicable reasons clear. Evaluate per output; pause the
decoder when all local outputs are paused. Preserve the group timeline while
hidden, then rejoin it. No sleep inhibition just to keep a wallpaper running.
Static poster fallback should use an ordinary still surface/native background
without continuously presenting frames.

| Signal | Windows adapter | macOS adapter | Linux adapters |
|---|---|---|---|
| Fullscreen / maximized / coverage | `SetWinEventHook` foreground/location/show/hide/minimize events; inspect visible un-cloaked windows on those events | Workspace activation events; own window occlusion notifications alone are insufficient for full global coverage. AX window notifications require Accessibility consent; complete event-driven coverage without AX is **UNVERIFIED** | GNOME bridge observes compositor windows; Plasma integration KWin hooks where supported; X11 EWMH/property/configure events; Sway IPC/Hyprland events only when supported |
| Lock/unlock | `WTSRegisterSessionNotification` session events | Workspace session-active/resign plus display sleep; exact secure-lock notification contract **UNVERIFIED**, conservative pause on session inactive | GNOME `ActiveChanged`; logind `LockedHint`/session state; compositor-specific lock signals may be unavailable |
| Display sleep / system sleep | `RegisterPowerSettingNotification`, `GUID_CONSOLE_DISPLAY_STATE`, `WM_POWERBROADCAST` | `NSWorkspace.screensDidSleep`/wake, system sleep notifications | GNOME `PowerSaveMode`/properties; logind `PrepareForSleep`; output power events where compositor exposes them |
| AC/battery / low battery | Power-setting notifications, battery percentage/status, one read on event | IOKit power-source run-loop callback, read power-source state on event | UPower `PropertiesChanged`, display-device percentage/warning level |
| Low power | `GUID_POWER_SAVING_STATUS` / supported power notifications | `ProcessInfo.isLowPowerModeEnabled`, power-state-change notification | PowerProfiles `ActiveProfile` property changes; desktop-specific alternatives |
| Gaming mode | one-mk gaming-state subscription; OS Game Mode integration **UNVERIFIED** | one-mk gaming state; native game-mode signal **UNVERIFIED** | one-mk gaming state; optional GameMode D-Bus integration **UNVERIFIED** |

Named Windows/macOS APIs are documented building blocks [S26, S27]; complete
coverage, timing and permissions remain **UNVERIFIED until OS tests**. Generic
Wayland intentionally has no universal "enumerate every other application's
windows" API; layer-shell is not a window-coverage API. Where unavailable,
doctor reports that auto-coverage pause is unsupported; do not poll screenshots
or silently claim the rule works. Compute union of opaque visible window
regions, not just "some window maximized"; workspaces, transparent windows,
panels and multi-monitor coverage matter. Unknown visibility uses conservative
policy, with owner-selectable play/paused fallback.

Efficient mode: offer 30 fps and 15 fps normalized representations. Merely
presenting every second frame of a 60 fps video may still decode all 60 frames;
it does not prove half the power. Keep preview hidden when not visible, mute
audio, stop pipeline clocks while paused, and suspend active synchronization
when every participant is paused. Playlists/day schedules use a next-deadline
timer, not one-second wall-clock polling; recompute on timezone/clock-change
events. Local timers do not change the monotonic playback epoch.

### Resource budget: calculations versus estimates

- Arithmetic: 3840×2160 NV12 (8-bit 4:2:0) is about **11.87 MiB/frame**;
  RGBA is **31.64 MiB/frame**. Eight NV12 decode surfaces are about 94.9 MiB
  before codec buffers, GPU alignment and output buffers. P010 costs more.
  Three independent decoder pools can roughly triple that part of memory.
- PROPOSAL/UNVERIFIED estimates: hardware-decoded 1080p30 engine might cost
  0.5–3% of one CPU core; 4K30 1–5%, roughly 100–300 MiB combined engine/GPU
  allocations depending on queue depth. Extra GPU/system power might be
  0.5–3 W for modest content, much more with a discrete GPU or software decode.
  These are budgeting hypotheses, not measurements of one-mk or the host.
- PROPOSAL: disable must add **zero periodic wakeups** and zero sustained
  daemon CPU relative to baseline; paused engine should be event-blocked with
  zero animation presents/decodes. RSS alone does not measure GPU memory.
  Report CPU as process CPU-time/wall-time and GPU power/memory separately;
  never compare platform task-manager percentages without normalization.
- Measure off/paused/playing at 1080p/4K and 15/30/60 fps, 1/2/3 monitors,
  hardware/software decode, AC/battery, with and without the app/tray open.
  Hold brightness/refresh/power profile constant in the lab. Measure a
  no-wallpaper baseline first. No such numbers have been measured here.

## 8. Content and gallery rights

### Video and animated images are first-release content

Own-file import supports MP4/MKV/WebM, then normalizes when necessary.
GIF/APNG/animated WebP need explicit frame-disposal/blending, alpha, delays,
finite/infinite loop semantics and dimension/allocation limits. Candidate
`image` decoding API is permissively licensed [S28]; verify pinned-version
support for each animated variant before adoption. Do not preload every frame:
an uncompressed minute of 1080p30 RGBA is about 13.9 GiB. Use bounded decode
checkpoints/cache or consented conversion to a common video representation.
Preserve transparency via native image frames or a background-composited
representation; ordinary H.264 does not preserve alpha.

Normalize pathological zero delays to a documented minimum, then use an
identical rational frame-time table/content duration on every device. At target
PTS, select the frame whose interval contains that time. Respect a finite-loop
file unless the owner chooses "loop forever". Transcoded images must preserve
the agreed visual timing, not whichever delay heuristic each OS happens to
use. Rate correction changes timeline mapping, not file frame metadata.

Shaders could be a later optional extra with a shared time uniform, but the
owner does **not** want shader animation in this implementation. No architecture
or synchronization requirement depends on shaders. Web pages/HTML/canvas would
require a browser/web view, conflict with one-mk's no-web-view direction, and
are explicitly out of scope. No embedded browser for the gallery either.

### Source-by-source terms audit

Distinguish permission to use a downloaded video as one's own wallpaper from
permission for **our app to operate a catalogue/download service**. Hosting
only JSON, or downloading directly to the end user, does not erase source
contracts, creators' rights, hotlink rules or standalone-distribution limits.
The following is a conservative integration recommendation, not a legal ruling.

| Source and evidence read | Usage/attribution and restrictions | JSON index, hotlinks, app download decision |
|---|---|---|
| **Pexels** API documentation + official wallpaper/terms help [S29] | Ordinary uses and API access are distinct; official API policy excludes wallpaper apps and competing libraries. Terms help specifically restricts standalone wallpaper distribution | **NO default integration.** No scraping, API key workaround or direct-CDN index. Written permission would be needed; a user's separately lawful file import is a different use |
| **Pixabay** Terms (updated 20 October 2026) + API docs [S30] | Content licence disallows standalone distribution, explicitly includes wallpapers; resizing/cropping is still standalone. Historic pre-9-Jan-2019 CC0 content has different copyright terms. No blanket author/release guarantee | **NO default integration.** API forbids permanent image hotlinking; that wording is not proof of video-hotlink permission. App-side video index/API clearance **UNVERIFIED**; do not recommend even historic CC0 as a feed without access clearance |
| **Mixkit** Video Free licence AND User Terms [S31] | Free licence permits broad reuse without mandatory attribution, but User Terms §9 restrict stock/inventory aggregation, competitive services and mass-download bots; Restricted licences differ | **NO default integration.** Free licence is not a blanket wallpaper-catalogue/API grant. Indexing/deep-link/download permission **UNVERIFIED**; seek written authorization |
| **Coverr** licence AND Terms [S32] | Free personal/commercial usage, attribution appreciated rather than generally mandatory; licence excludes compiling a similar service and FAQ prohibits redistribution/video-providing services | **NO default integration.** Do not assume advertised API equals approval for a wallpaper gallery. Need written product-specific permission and applicable API contract |
| **Wikimedia Commons** reuse guidance [S33], underlying CC terms [S34] | Per-file licence/author governs. Public-domain/CC0, CC BY, CC BY-SA and others differ. Link to licence, credit actual creator not merely uploader; mark adaptations; share-alike applies to adapted media, not automatically the Rust program | **YES, narrowly curated verified CC0/public-domain files first.** Guidance explicitly allows download/direct linking (hotlinking possible but discouraged). Download locally on click, don't continuously stream. Reuse factual index metadata, not copied site descriptions. Respect API access rules [S35] |
| **Internet Archive** live Terms URL [S36] | Terms page returned only a JavaScript-required shell here; no substantive current contract retrieved. Item metadata is not a licence guarantee | **UNVERIFIED: NOT an approved feed.** Recheck actual terms, metadata API/download policy and each item's rights before recommending; no "Archive = public domain" assumption |
| **NASA media** Brand Center images/media guidance [S37] | NASA-created media is generally not copyrighted in the US, but third-party credited work, music, logos/insignia, identifiable people and endorsement/publicity restrictions remain. Not all NASA-hosted assets are government-created | **Conditional candidate, not release-one default.** Only individually proven NASA-created/public-domain items; agency credit/provenance and non-endorsement text. Permission for our precise automated/API/catalogue delivery remains **UNVERIFIED**; use only after checking the specific host's access/API rules |

**Approved initial gallery:** small manually audited Commons items with explicit
CC0 or unambiguous public-domain status for relevant markets, or creators who
give us an explicit CC0 grant and a documented download-host permission. No
actual videos have been selected or cleared by this document. Do not label any
specific file approved merely because it is on Commons. CC0/public domain do
not clear trademark/publicity rights; prefer scenery/abstract scenes without
people/logos. Optional CC BY items can follow after attribution/adaptation
tests; do not initially ingest BY-SA, NC, ND, GFDL or ambiguous files simply to
inflate catalogue size. This is a stricter product policy than the range of
licences Commons permits.

For Wikimedia API use [S35], the policy read requires an identifying user
agent/contact, respect for throttling and content licences, and no disruptive
traffic or request splitting to hide excessive use. It prohibits white-label
API clients that obscure Wikimedia as the provider; identify Wikimedia in the
gallery and never guarantee its API availability. Cache metadata and back off;
no bulk crawler or per-frame remote request. Endpoint-specific numeric quotas
are **UNVERIFIED** and must be checked before enabling a feed. Do not hotlink previews unless cleared; no
third-party thumbnail is bundled in the installer either. Generate a local
preview after an authorized download, or fetch a cleared thumbnail on demand.

### Small JSON catalogue, clear rights, real takedowns

Host only bounded signed JSON: item ID, source landing page, allowed download
URL/host, title (original/factual), author, licence identifier/version and
licence URL, copyright/credit line, required attribution, rights evidence,
review date, size/duration/codec hints and byte checksum. Retain immutable
licence evidence in the catalogue-maintenance process; **do not host videos**
in repo, installers or catalogue infrastructure. Checksums require one
authorized bounded download during curation, not guessing a CDN URL. Handle
source revisions as new hashes; reject unexpected bytes. A source's hash may
need mapping to BLAKE3 when it publishes another digest algorithm.

The native gallery must show **licence and author per wallpaper** before
download and in Library/About after it, with source and licence links, changes
made by transcoding, a copyable credit line, byte cost, and a Report rights issue
action. If metadata licence text is itself share-alike, comply or write our own
factual title/summary; do not copy a site's entire description/database.
Retain attribution/provenance alongside locally copied/paired media.

Publish a contact/takedown route. Proposal: acknowledge within 2 business days,
temporarily delist on a credible claim, investigate provenance, notify affected
users and publish a signed index tombstone. Cache the last valid index for
offline use, but stop new downloads of revoked items once notified. Do not
silently delete owners' own files; offer removal of affected cached media and
explain revocation. Re-review source terms on catalogue updates. Terms changes
or a source shutdown disable that feed without affecting own-file playback.
If no cleared items exist, ship an honest empty gallery with import guidance,
not unlicensed placeholders; first-release gallery acceptance still requires
real cleared download items before claiming the gallery is ready.

## 9. Dependencies and licence decisions

Keep one-mk-authored Rust under `MIT OR Apache-2.0`. Dependencies keep their
own licences; a permissive source tree is not a promise that every distributed
binary/dependency can be used without other obligations. No GPL library is
approved for linking into one-mk binaries. Prefer OS frameworks already on the
machine and a narrowly audited Linux runtime. All pinned transitive dependency
licences and actual codec build options remain **UNVERIFIED until lockfile,
build logs and shipped artifacts are audited**. Do not approve an entire
plugin directory based on one package's top-level licence.

| Candidate | Evidence/status | Compatibility and packaging action |
|---|---|---|
| Windows Media Foundation/D3D/DirectComposition; Apple AppKit/AVFoundation/VideoToolbox/Metal | OS APIs [S04, S05, S07–S12] | Use installed frameworks under platform SDK/runtime terms; no borrowed Muro/Aura code; codec extension/patent review separate |
| Rust `windows` bindings | MIT licence read [S38] | Compatible; retain notices; review selected generated bindings/dependencies |
| Rust `objc2` | MIT manifest read [S39] | Compatible; generated framework crates independently checked before pinning |
| Rust `image` | MIT OR Apache-2.0 manifest [S28] | Compatible; enable only required GIF/PNG/WebP support; audit decoder dependencies/features |
| Rust `wayland-client` | MIT manifest [S40] | Compatible; protocol-binding crates/GL bindings still need exact-version review |
| Rust `gstreamer`/gstreamer-rs | MIT OR Apache-2.0; runtime LGPL-2.1-or-later [S41] | Rust wrapper compatible; dynamically ship/use replaceable LGPL runtime with notices/source compliance; audit selected plugins and codec libraries individually |
| GStreamer native runtime | LGPL framework; plugins can have other requirements [S22, S41] | No blanket "all plugins are LGPL" claim; package an allowlist, expose runtime decoder/build details in doctor |
| FFmpeg/libav* | LGPL-2.1+ baseline, GPL parts/configuration or version3 can change terms; nonfree builds may be unredistributable [S42] | Optional restricted import/transcode helper; prefer audited LGPL configuration, never `--enable-gpl`/`--enable-nonfree` in bundled permissive-linked path |
| libmpv | GPL-2.0-or-later default; LGPL-2.1-or-later build possible, dependencies can change final obligations [S43] | Not default; reject unknown build licence. `-Dgpl=false` alone is not evidence every linked library is LGPL-compatible |
| mpv/mpvpaper separate executable | mpv build-dependent above; mpvpaper GPL-3.0 [S20, S43] | Optional external experiment only. No source copied. Distribution of their binaries still requires corresponding licences/source/notices |
| `wgpu` (architecture B only) | MIT OR Apache-2.0 in upstream workspace [S44] | Compatible renderer candidate, not a decoder or GNOME-background API; audit backends/transitives |
| GNOME bridge / Plasma bridge | No implementation or dependency lock yet | **UNVERIFIED legal gate:** independently authored code; examine Shell/GJS/Clutter and selected Qt/KDE modules and plugin-linking obligations. A plugin loaded into a GPL host is not made licence-free by a permissive manifest |

### What dynamic libmpv linking actually means

Dynamic loading (`dlopen`/DLL loading) does **not** make GPL obligations vanish.
Linking to a GPL libmpv build is not approved for a distributed one-mk binary
under only its permissive licensing model. GPL-2-only combinations also raise
Apache-2.0 compatibility concerns; do not solve this by claiming the default
"GPL-2+" label automatically re-licenses one-mk. A GPL-compliant distribution
would need a deliberate different combined-work licensing review, not just
an About-box acknowledgement.

With a genuinely LGPL-2.1+ libmpv build and compatible dependencies, dynamic
linking can preserve our own code's permissive licence. Preserve notices,
provide the LGPL text and required library source/modifications, allow user
replacement/relinking with a modified compatible library, and do not prohibit
reverse engineering for debugging library modifications. Static linking adds
relinkable-object/source obligations; avoid it. Installer signatures and app
bundles need a practical compliance/replacement review. LGPL-2.1 sections 4–6
were read from the installed full licence text [S45]: section 6(b) permits a
suitable replaceable shared-library mechanism; distributing the library itself
also brings its source obligations. This is not an obligation to relicense our
own independently authored source as GPL. Exact installer compliance remains
**UNVERIFIED pending the actual binary/dependency audit**, not a legal sign-off.

Spawning an independent player through command arguments/ordinary IPC is a
different architectural boundary, not libmpv linking. It ordinarily lets
one-mk remain independently permissively licensed, but distribution of that
GPL player still has obligations; an intimate combined-work protocol also
needs review. "It is a subprocess" is not a universal legal exemption. Using
an installed OS media API avoids bundling libmpv/FFmpeg on Windows/macOS.

## 10. Architecture options and selection

Effort estimates are **UNVERIFIED planning ranges**, one experienced engineer's
focused engineer-weeks, including basic integration tests but not waiting for
hardware/access/legal approvals. All options still need the shared geometry,
authenticated media flow, image timeline and clock protocol. A two-week
mpvpaper demo is not an enterprise cross-platform implementation.

| Architecture | Mechanism and coverage | Main risk / licences | Estimate |
|---|---|---|---|
| **A — selected: native adapters** | Shared Rust scene/clock/image scheduler; GStreamer/GL Linux renderer, MF/D3D Windows, AVFoundation/Metal macOS. Layer-shell plus separate GNOME/Plasma presenters; cooperative X11 targets only. One decode per device/N crops | Separate adapter maintenance and cross-process texture sharing; LGPL Linux runtime, permissive wrappers, OS APIs. GNOME/Plasma integration gate | **28–42 weeks** through qualified release one |
| **B — common GPU renderer** | `wgpu` textures/crops across desktop-level surfaces, OS or library decode plus image path. Same platform integrations still required; does not make Shell/Plasma ordinary Wayland clients | Native video-buffer import into wgpu, copies, driver limits and codec distribution. wgpu permissive, decoder-specific licence | **36–54 weeks**; potentially better later shared render logic |
| **C — spawn player** | mpv/mpvpaper on supported Linux layer-shell/X11; custom host windows/players on Windows/macOS; JSON IPC timing/crops where supported | Fast proof, N processes/decoders likely, player flags/security/build variation, GNOME/Plasma unsolved by process launch. GPL executables separate, not linked | **6–10 weeks** limited prototype; **22–36** attempting release scope, may still fail one-decoder requirement |

A is the recommendation: smallest distributable media dependency burden on
Windows/macOS and explicit control over resource/clock contracts. B is a
reasonable future refactor only if shared rendering materially reduces risk.
C is a disposable experiment if a player is already installed, not the first
release foundation. None provides universal X11 desktop-manager compatibility.

On this Linux box, pure scene/image/clock logic, protocol failure tests,
software rendering and private Xvfb can be tested once implementation exists.
Nested/headless Wayland, other desktops and hardware decode need additional
lab prerequisites. Windows adapters can be compile-checked only if targets/
toolchains exist; macOS Rust checks similarly cannot substitute for Apple
framework link/sign/notarization tests on macOS. Availability of those tools
here is **UNVERIFIED**; no compile was run in this research. Platform runners
and actual Windows/Mac computers are required before claiming support.

### Phased work: all three phases precede release one

**Phase 1 — internal local-canvas and integration-risk milestone (8–12 weeks).**
Build pure bounded scene geometry, deterministic video/image timeline,
injectable monotonic clock and renderer/decoder trait from day one. Build
Linux one-decode/N-crop pipeline in disposable displays; begin GNOME 50 and
Plasma bridge spikes immediately rather than discovering blockers at the end.
Compile native-adapter interfaces/stubs where toolchains allow.

Acceptance: negative/gapped/portrait/mixed-scale golden crops pass; GIF/APNG/
WebP frame timing/disposal/alpha fixtures pass; one verified decode instance
feeds two virtual outputs; off-state daemon unchanged; malformed media bounded
and isolated. GNOME spike must prove below-DING placement and external-renderer
timing without decoder code in Shell, in a disposable session. If a spike
fails, re-estimate explicitly. **Internal only, not a public single-PC release.**

**Phase 2 — internal multi-device synchronization and native adapters
(10–16 weeks).** Add authorized scene manifests, layout consistency checks,
cache-purpose transfer negotiation, clock bursts, drift correction, pause/
resume semantics and diagnostics. Two isolated Linux “computers” on loopback
exercise the real paired QUIC route with null input. Finish Windows/macOS
adapters and perform first real-device vertical slices. No WebView or shader
shortcut for synchronization; test video and animated images separately.

Acceptance: two processes with distinct virtual displays show disjoint slices
of the same movie/image animation, not copies of the full frame; meet loopback
sync target; survive sleep simulation/rejoin/disconnect/coordinator loss,
corruption/disk-full/slow decode; no transfer can delay input beyond agreed
baseline bounds. Test native Windows/Mac scheduling and icon placement on
real hardware. A compile-only platform remains labelled untested and blocks
the claimed all-three-platform release.

**Phase 3 — first shippable release qualification (10–14 weeks).** Native
Wallpaper tab/tray/CLI/doctor, own-file and safe URL imports, audited Commons
gallery, playlists/day automations, event-driven pause, signing/packaging,
licence/SBOM/security review and full physical matrix. Qualification may run in
parallel with adapter completion, but none is skipped.

Acceptance: real Linux↔Windows↔macOS scene across at least three computers,
multiple physical monitors and qualified wired/Wi-Fi sync results; real video
plus all three image formats; cleared gallery download + licence display;
bounded crash/failure behavior; off-state CPU/wakeup proof; 24-hour soak;
manual scripts signed off. Release notes name tested desktop/OS/GPU versions
and limitations; mark other combinations experimental/unsupported, never just
"cross-platform" based on compilation. A subset release needs an explicit
owner scope change, not silent deferral of multi-device or an OS.

## 11. Fit with one-mk

PROPOSAL crate split: `crates/wall-core` for pure scene/image-time/clock math
and bounded schemas, `crates/wall` for `one-mk-wall` OS playback/presenter
adapters, plus a restricted import/decode helper mode if useful. Separate
bridge artifacts under packaging; no media runtime dependency from
`crates/one-mk`, `proto`, `net` or input libraries. `wall-core` must not turn
`proto` into a media decoder dependency. Reuse existing QUIC/auth/transfer
services through a bounded active subscription, not a second unauthenticated
network listener. Peer private keys remain with their existing owner process.

Potential local control requests: `WallpaperStatus`, `WallpaperAuthorize`,
`WallpaperStart`, `WallpaperPause`, `WallpaperStop`, `WallpaperImport`,
`WallpaperScene`, `WallpaperSubscribe`. Separate user-local IPC protected by
same-user credentials/ACLs controls the engine; remote peers only get negotiated
wallpaper schemas, never the generic local control channel. New protocol caps
could include `wall_scene_v1`, `wall_cache_v1`, `wall_clock_v1`; a namespace or
stream-kind/version strategy needs compatibility tests against current v2
peers. No new unconditional heartbeat in the daemon.

Proposed CLI, not existing commands:

```text
one-mk wallpaper doctor
one-mk wallpaper import <file-or-https-url>
one-mk wallpaper gallery list
one-mk wallpaper gallery download <item-id>
one-mk wallpaper scene preview
one-mk wallpaper start <media-id> --scope paired --fit fill --gaps include
one-mk wallpaper pause --local
one-mk wallpaper pause --group
one-mk wallpaper status --json
one-mk wallpaper stop
```

Persist `[wallpaper]` separately/versioned in `settings.toml`: disabled default,
coordinator consent/member list, scene ID/layout revision, media IDs, fit/gaps,
pause reasons/policies, hardware-required choice, fps/quality, caps, cache quota,
playlist/schedule and explicit autostart. Keep media/cache in standard per-user
data/cache directories in the eventual app, not executable/source directories.
This document authorizes no writes there now. Separate policy and ephemeral
clock/PTS; never persist peer-supplied absolute paths or network URL credentials.

Add a **Wallpaper tab** to the existing native egui app with Library/Gallery,
rights labels, import progress, scene preview and a sync-quality view. Use
existing Layout tab placements; add an optional wallpaper overlay there rather
than a conflicting second layout editor. Warn about unspecified offsets,
overlap, reserved/offline rectangles and maximum decode size. Preview only on
explicit demand, stop when hidden. Tray entries: Play/Pause, Next, current
scene, quality/degraded status and Stop; no always-running new tray process.
Day automation chooses one shared scene using the coordinator's timezone;
clock/DST changes change future schedules, not active PTS.

Doctor must state: backend selected, compositor/version, required bridge
installed/enabled/protocol version, icon-preservation qualification, decoder
actually selected/hardware status, format/codec availability, licence/build
identity, GPU limits, cache bytes, permissions, local/remote scene versions,
clock uncertainty/PTS error, unsupported pause rules, and actions that need
owner consent. Never turn on Accessibility or install an extension silently.

Packaging: optional `one-mk-wall` payload/component so existing headless daemon
installs stay small. `.deb`: separate optional wallpaper package with exact
GStreamer/plugin runtime dependencies and optional desktop bridge packages;
do not add all media plugins to core Depends. AppImage: bundle audited runtime
and scanner/allowlisted plugins or clearly require host components; test driver
ABI/library replacement and report size. Windows installer: optional signed
user-scoped wall/helper component, use OS MF; detect missing codec components,
never buy/download paid extensions automatically. macOS bundle: signed helper,
native framework linkage, notarization, minimal entitlements and universal/arch
coverage. Inspect licensing for any Qt/bridge payload before distribution.

Uninstall removes only our autostart/bridges/settings on explicit request,
never other wallpaper software or user source videos; preserve static fallback.
Update packaging checks, installer smoke tests, README platform table,
FEATURE-PLAN, dedicated user guide, gallery rights policy, protocol/schema
documentation and troubleshooting/QA matrix. Do not report these changes as
implemented by this research document.

## 12. QA-agent executable plan and release evidence

**Mandatory quality bar:** polished native controls, actionable errors, atomic
configuration updates, bounded recovery, no desktop/input regression, real
end-to-end tests, and honest platform labels. Unit tests and compilation are
necessary but not sufficient. The following is a future execution plan; all
playback tests are **NOT RUN** in this task.

### Lab rules and prerequisites

Use a disposable VM/session for desktop integration, never the owner's live
GNOME session. No input backend except the explicit null test backend. Use
private user state, trust store, cache and sockets; do not reuse real paired
identities. Keep generated fixtures small under a worktree-owned lab directory,
not a large `/tmp` RAM-disk video. Builds/tests use `-j 2`; never run
`cargo fmt`. Installing a missing compositor/codec belongs to a separately
approved lab setup, not this task. Inspect supported test-CLI options before
launching; the proposed wallpaper commands above do not yet exist.

1. **Unit/golden stage:** run the eventual `cargo test -p one-mk-wall-core -j 2`
   (package name to match implementation) and bounded schema/property tests.
   Cases: negative origins; holes; 1/1.25/1.5/2 scales; 90/270° portrait;
   mirror/overlap; graph cycles; side-only layouts; huge/overflow rectangles;
   source rotation/SAR; Fit/Fill/Stretch/Centre; partial letterbox and gap masks.
   Generate original numbered grid/diagonal/moving-bar fixtures in tests, no
   third-party assets. Compare each crop against a CPU reference at fixed PTS;
   exact pixels for nearest sampling, declared tolerance for GPU filtering.
2. **Images:** handcrafted GIF/APNG/WebP fixtures with disposal/background/
   previous, partial frames, alpha, varying/zero delay, finite loop and truncated
   data. Compare canonical frame timeline and every composited frame on all
   backends; random-access late join must select the same frame. Allocation
   bombs hit limits before memory growth, and malformed files never reach Shell.
3. **Private X11 display:** use installed Xvfb with private authorization and
   `-nolisten tcp`, e.g. a harness-selected free display and bounded 1920×1080
   screens. Add a tiny test WM/desktop fixture where needed. Multiple Xvfb
   screens are separate X screens, **not automatically RandR-arranged monitors**;
   emulate monitor rectangles in the harness or use a RandR-capable lab X server.
   Capture only that server's wallpaper windows at chosen PTS; stitch reference
   crops and compare. Run focus/input-pass-through and restart tests. This does
   not qualify GNOME/KDE/Xfce/Cinnamon/MATE integration.
4. **Private Wayland displays:** after approved prerequisites, launch two
   independent headless wlroots/Sway lab compositors with two outputs each,
   software rendering where necessary, isolated `XDG_RUNTIME_DIR`, distinct
   `WAYLAND_DISPLAY`, private D-Bus and no inherited owner's display socket.
   A harness environment can set `WLR_BACKENDS=headless`,
   `WLR_HEADLESS_OUTPUTS=2`, `WLR_RENDERER=pixman`; exact compositor/backend
   support is **UNVERIFIED until installed**, inspect its help. Weston headless
   alone must not be assumed to support wlr-layer-shell. Use compositor frame
   capture/protocols available in that lab, not screenshots of the live desktop.
5. **Two “computers”, real transport:** launch two daemon instances in their
   documented null-backend test mode, separate ports/configs/trust/caches,
   pair test identities over loopback and place B at an explicit offset from A.
   Attach one wall engine to each private display. Test third process for leader
   conflict/offline behavior. Exercise existing file-stream resume and final
   hash, not mocked file copies. Run once with video and once for each animated
   format. Use a common future epoch and capture both surfaces at defined times.
6. **Clock simulation and measurement:** injectable clocks test ±100 ppm,
   ±2-second initial offset, monotonic reset, suspend, jitter, reordering, rate
   clamp and loop wrap. Apply network impairment only inside a disposable
   namespace/VM, never host Wi-Fi/routes. Test symmetric and asymmetric delay,
   0/1/5% loss, transient outage and bandwidth contention; state that loopback
   cannot reproduce real Wi-Fi contention/scanout.
7. **Measure the display, not just the algorithm:** log session ID, local and
   mapped presentation time, frame ID/PTS, clock estimate/uncertainty, decode
   delay/drop count. On one host the monotonic clock provides independent ground
   truth; inject artificial per-process clock transforms to avoid a vacuous test
   where both simply read the same clock. Capture numbered frame IDs in both
   render targets at a shared sampling instant and compute P50/P95/P99/max
   presented-PTS error after 10-second warmup over ≥30 minutes. Fence-complete
   offscreen capture measures rendering, not physical scanout; label it so.
   On real machines film both monitors at high frame rate with an original
   moving-bar/frame-counter clip; quantify frame mismatch and seam displacement.
8. **Security/failure stage:** unpaired/refused/forgotten coordinator, replayed
   generation, unauthorized media ID/path, traversal/symlink cache escape,
   hostile metadata, quota/rate violation, malformed/oversized movie, URL to
   loopback/private host/redirect/rebinding, spoofed catalogue signature/hash,
   revoked item and interrupted import. Assert refusal, bounded RSS/CPU/disk,
   preserved previous scene and no arbitrary file open. Crash decoder/engine/
   bridge; desktop/input must continue. Examine child handles/permissions and
   inability to read pairing keys or open network connections.
9. **Power/soak stage:** 10-minute baseline/off/paused CPU-time and wakeup samples;
   24-hour active/looping test with output hotplug, simulated battery/power
   events, playlist rollover and one engine restart per hour. No growing queue,
   bounded cache/RSS/GPU allocations, no periodic daemon wallpaper wakeups when
   off, no resource leaks after 100 enable/disable cycles. Event-driven pauses
   should stop decode/presentation within 1 second (PROPOSAL target); no polling
   loop permitted to make the test pass. A pause reason must survive unrelated
   reasons clearing.
10. **UX/packaging stage:** fresh installs/updates/uninstalls on each supported
    OS, missing runtime/bridge/codec, readonly/full cache, offline gallery,
    keyboard navigation, accessibility labels, reduced-motion preview policy,
    cancellation, corrupt settings rollback, no autoplay before consent,
    source/author/licence display, signed update failure and safe-mode recovery.

### Gate thresholds and report format

Gate on zero crash/desktop-icon/input regressions in supported configurations,
one decode instance/device on normal path, capped resources, sync targets in
section 5, correct crops/images and no unapproved/licence-ambiguous dependencies
or gallery items. Input-latency nonregression target: transfer/active-wallpaper
P95 increase ≤1 ms in the controlled loopback lab (PROPOSAL, tune with baseline
noise); real remote-input measurements must also be reported, not inferred.
Any threshold change needs owner review and published rationale.

QA records: commit/build hashes, OS/desktop/GPU/driver/codec versions, selected
backend and actual decode mode, monitor map/scales/refresh, input baseline,
fixture hash, clock/network scenario, commands, logs, screenshot diffs, sync
distribution and resource graphs/data, passed/failed/skipped with reason.
Mark `compiled only`, `offscreen tested`, `virtual-display E2E` and
`physical-device E2E` separately. No passing Linux simulator result certifies
Windows/macOS, secure desktop, real hardware power or physical seam quality.

## 13. Owner manual scripts for real desktops

These scripts are for a consenting owner/tester **after** the feature exists,
in a dedicated test session with saved work. They are not actions performed
during this research. Prepare an original grid/moving-bar movie plus small
GIF/APNG/WebP fixtures; record versions and keep a static fallback. Every run:
use two differently sized/scaled monitors where possible, show desktop icons,
select continuous canvas, inspect grid alignment, try all four scaling modes,
check icon clicks/context menus/drag and task switching, then pause/stop and
verify the original desktop returns. Capture failures before retrying.

| Platform / desktop | Manual sequence and pass condition |
|---|---|
| Windows 11 24H2 **and current supported Windows builds**, Windows 10 only if still claimed | Enable optional wall component; ensure window stays under icons, absent from Alt-Tab and never steals focus. Move a monitor left/above primary, change DPI/portrait, fullscreen game, maximize/minimize, switch virtual desktops, lock/unlock, sleep/wake, AC/battery and efficient mode. In a disposable session restart Explorer; wallpaper recovers or fails static without icon loss/refresh storms. Test missing HEVC/Media Feature Pack and mixed-GPU paths. Record OS build, not merely "Windows 11" |
| macOS Intel and Apple silicon | Desktop-level placement under Finder icons; Spaces, Mission Control, Stage Manager and Show Desktop; fullscreen Space, hotplug, Retina scaling/portrait, low-power/battery, lock, display sleep. No wallpaper on secure login screen, no permissions silently enabled. Deny Accessibility and check honest unsupported coverage rule. Test native-decode capability failure and signed helper update. Do not claim Muro-style lock/saver support |
| GNOME **50.1 Wayland**, then each additionally claimed 45–49 version | Use a disposable login/VM, explicitly install/enable matching bridge, keep DING/desktop-icons extension enabled. Verify icons above video, Overview/workspace switching, top bar/dock, fullscreen/maximized/partially covered windows, lock/unlock and display sleep. Disable bridge: no black desktop, orphan surface, key grab or crashed Shell. Upgrade/mismatched bridge fails static with doctor guidance; never disable version validation |
| KDE Plasma Wayland **and X11 if claimed** | Select our actual wallpaper plugin in Desktop Settings; verify Folder View/icons remain usable and layer-shell test surface is not mistaken for a working plugin. Test containment/activity/virtual desktop changes, edit mode, compositor restart in disposable session, screen hotplug, power pause and plugin disable/update. Decoder crash must not crash plasmashell |
| COSMIC Wayland | Verify advertised layer-shell and interaction with its own background service. Run common monitor/icon/fullscreen/power tests; if coverage events unavailable, doctor must say so. Stop restores existing background without disabling desktop services |
| Sway / Hyprland (separate runs) | Capability/IPC discovery, two outputs, scale/transform, output unplug, workspaces/fullscreen, session lock, manual pause with no window-coverage IPC. No reserved work area or focus/input interception; own background-client conflict yields honest setup guidance |
| GNOME X11 (older version only) | Test Shell/background/icon integration on an explicitly supported older GNOME release, not GNOME 50. Root-pixmap success is insufficient; icons and Shell background must coexist. Otherwise mark unsupported |
| Xfce X11 | Leave xfdesktop/icons on; verify actual cooperative presentation. If video is hidden or hides icons, fail the desktop-support gate rather than turning xfdesktop off |
| Cinnamon X11 | Leave Nemo desktop on; verify wallpaper/background ownership, icon interaction, lock, hotplug and restart. Version-specific failure means unsupported, not a root-window workaround claim |
| MATE X11 | Leave Caja desktop on; same visibility/icon/lock/hotplug tests. Record Caja/WM/compositor versions and transparency settings; do not alter them automatically |

**Cross-computer script (mandatory for release one):** pair the test Linux,
Windows and Mac; opt in each receiver; give every device explicit offsets;
place a moving bar across two physical seams and include a vertical/negative-
coordinate monitor. Import once, confirm cache hashes and all three image
formats, prepare/start the same scene, and film numbered frames on both sides
of each seam. Record wired and Wi-Fi distributions. Close one lid, lock another,
unplug Wi-Fi, throttle a test decoder, change layout, fill a test cache quota,
and rejoin late. Others must continue; returners join the current slice/time
without resetting the group. Forget a device and prove it can no longer make
remote changes. Stop everywhere; verify KVM, clipboard, transfers, desktop
icons, previous background and off-state CPU. A seam visibly jumping is a
measured limitation to report, never hide behind a "synced" badge.

## 14. Decisions still needed from the owner

The platform/content/gallery/multi-device answers are **settled**: all three
OSes, video + animated images, own files + URL + gallery, no shaders/web pages,
and multi-device in the first shippable version. Do not reopen them as excuses
to ship a single-computer feature.

1. Approve native-adapter architecture A and **28–42 engineer-week** planning
   range, with GNOME/Plasma bridge feasibility as early stop/re-estimate gates?
2. Which exact Linux desktop versions and Windows/macOS minimum releases must
   pass release-one qualification, and who supplies real Windows/Mac hardware?
   In particular, must Xfce/Cinnamon/MATE be supported rather than honestly
   marked unsupported if icon-preserving integration is not available?
3. Is an explicit optional GNOME Shell/Plasma plugin acceptable, including
   privileged-host maintenance and any separately licensed bridge obligations?
4. Accept best-effort sync with proposed 50 ms P95 qualified Wi-Fi target and
   visible seam risk, or require tighter physical measurements that may change
   scope/hardware? Default to included gaps and reserve offline rectangles?
5. Accept proposed file/cache/resolution caps, SDR-only release one, hardware-
   required video default and optional smaller/software representation consent?
6. Accept a small CC0/public-domain Commons-only initial gallery rather than
   restricted stock feeds, with no videos bundled and a real takedown contact?
7. Choose coordinator authority and local-vs-group pause defaults; approve local
   explicit opt-in even for paired computers and no automatic leader takeover?
8. Is optional Accessibility permission on macOS acceptable for reliable
   coverage pause, or should unavailable coverage be shown as a limitation?

## Sources (primary unless explicitly noted)

Repository and host evidence is listed in section 1. These links are evidence,
not permission to copy implementation code. Only Muro/Aura/Hanabi READMEs and
licensing metadata were used; not their implementation files.

- **S01** Muro README, feature claims and licence: <https://github.com/MrRockySL/Muro#readme>.
- **S02** Aura README, desktop/feature claims and GPL label: <https://github.com/antwny/aura#readme>.
- **S03** Hanabi README, external-renderer/Shell integration pattern: <https://github.com/jeffshee/gnome-ext-hanabi#readme>.
- **S04** Microsoft `SetParent`, including DPI caveats: <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setparent>.
- **S05** Microsoft DirectComposition best practices/device loss: <https://learn.microsoft.com/en-us/windows/win32/directcomp/best-practices-for-directcomposition>.
- **S06** Microsoft Windows 11 24H2 resolved issues, wallpaper customization apps: <https://learn.microsoft.com/windows/release-health/resolved-issues-windows-11-24h2>. Does not document WorkerW as a stable API.
- **S07** Microsoft supported Media Foundation formats: <https://learn.microsoft.com/windows/win32/medfound/supported-media-formats-in-media-foundation>; MKV container support: <https://learn.microsoft.com/en-us/windows/win32/medfound/mkv-support>.
- **S08** Apple NSWindow collection behavior: <https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct>. Exact installed-OS stacking behavior not tested.
- **S09** Apple AVSampleBufferDisplayLayer controlTimebase: <https://developer.apple.com/documentation/avfoundation/avsamplebufferdisplaylayer/controltimebase>.
- **S10** Apple scheduled AVPlayer rate/time: <https://developer.apple.com/documentation/avfoundation/avplayer/setrate(_:time:athosttime:)>.
- **S11** Apple VideoToolbox hardware query: <https://developer.apple.com/documentation/videotoolbox/vtishardwaredecodesupported(_:)>.
- **S12** Apple Screen Saver framework: <https://developer.apple.com/documentation/ScreenSaver>.
- **S13** EWMH desktop window type: <https://specifications.freedesktop.org/wm-spec/latest/ar01s05.html>.
- **S14** GNOME 50 extension porting guide, including X11 removal: <https://gjs.guide/extensions/upgrading/gnome-shell-50.html>.
- **S15** KDE Plasma plugin documentation: <https://develop.kde.org/docs/plasma/>. Efficient external texture bridge is not established by this page.
- **S16** Xfce desktop-manager role: <https://xfce.org/projects>.
- **S17** Linux Mint Nemo desktop-icon role: <https://community.linuxmint.com/software/view/nemo>.
- **S18** MATE release note on Caja background/icon responsibility: <https://mate-desktop.com/id/blog/2024-02-27-mate-1-28-released/>; Caja documentation: <https://wiki.mate-desktop.org/mate-desktop/applications/caja/>.
- **S19** wlr-layer-shell protocol definition: <https://wayland.app/protocols/wlr-layer-shell-unstable-v1>.
- **S20** mpvpaper README/GPL-3.0 metadata and linked man page: <https://github.com/GhostNaN/mpvpaper>. No executable installed here.
- **S21** GNOME 45 ES module transition: <https://gjs.guide/extensions/upgrading/gnome-shell-45.html>; extension creation/installation: <https://gjs.guide/extensions/development/creating.html>.
- **S22** GStreamer application/plugin licensing FAQ: <https://gstreamer.freedesktop.org/documentation/frequently-asked-questions/licensing.html>; synchronization model: <https://gstreamer.freedesktop.org/documentation/additional/design/synchronisation.html>.
- **S23** NTP four-timestamp model, RFC 5905: <https://www.rfc-editor.org/info/rfc5905/>. Not evidence of wallpaper Wi-Fi accuracy.
- **S24** FFmpeg security fixes: <https://ffmpeg.org/security.html>.
- **S25** mpv security issue CVE-2018-6360: primary upstream advisory/commit not retrieved, so exact historical exploit specifics **UNVERIFIED for this review**. Current player manual is the primary hardening reference: <https://mpv.io/manual/stable/> (configuration, scripts, network protocols, IPC).
- **S26** Microsoft power-setting events: <https://learn.microsoft.com/en-us/windows/win32/power/registering-for-power-events>; other signal APIs named in section 7 require exact target-SDK validation before use.
- **S27** Apple NSWorkspace sleep notifications: <https://developer.apple.com/documentation/appkit/nsworkspace>; power state notification: <https://developer.apple.com/documentation/foundation/nsnotification/name-swift.struct/nsprocessinfopowerstatedidchange>.
- **S28** image crate manifest licence/features: <https://raw.githubusercontent.com/image-rs/image/main/Cargo.toml>; animation APIs: <https://docs.rs/crate/image/latest>.
- **S29** Pexels API docs: <https://www.pexels.com/api/documentation/>; official wallpaper FAQ: <https://help.pexels.com/hc/en-us/articles/4405588861721-Can-I-use-the-API-as-a-wallpaper-app>; official terms explanation: <https://help.pexels.com/hc/en-us/articles/900005880463-What-are-the-Terms-and-Conditions>. Full separate API Terms URL fetch failed; no approval inferred.
- **S30** Pixabay full Terms: <https://pixabay.com/service/terms/>; API hotlink/access policy: <https://pixabay.com/api/docs/>.
- **S31** Mixkit licence: <https://mixkit.co/license/modal/videoFree/>; User Terms §9: <https://mixkit.co/terms/>.
- **S32** Coverr licence: <https://coverr.co/license>; Terms of Use: <https://coverr.co/terms>.
- **S33** Commons external reuse/download/linking guidance: <https://commons.wikimedia.org/wiki/Commons:Reusing_content_outside_Wikimedia>.
- **S34** CC0 full legal code read: <https://creativecommons.org/publicdomain/zero/1.0/legalcode>. Waiver/fallback permits broad copyright reuse but does not clear all third-party rights or imply endorsement. No specific file has been selected or cleared; per-item clearance remains **UNVERIFIED**. Check the applicable full CC BY/CC BY-SA instrument before admitting such items later.
- **S35** Wikimedia API access policy read: <https://foundation.wikimedia.org/wiki/Policy:Wikimedia_Foundation_API_Usage_Guidelines> (page last edited 21 February 2026). Identifying client/contact, traffic/throttling and content-licence requirements; no provider-obscuring white-label API. Endpoint-specific numeric quotas remain **UNVERIFIED** and are an enablement gate.
- **S36** Internet Archive Terms: <https://archive.org/about/terms.php> redirects to <https://archive.org/about/terms>; substantive terms inaccessible in this research.
- **S37** NASA media/branding guidance: <https://www.nasa.gov/nasa-brand-center/images-and-media/>. Host/API-specific app download clearance not verified.
- **S38** Windows Rust bindings MIT text: <https://raw.githubusercontent.com/microsoft/windows-rs/master/license-mit>.
- **S39** objc2 MIT manifest: <https://raw.githubusercontent.com/madsmtm/objc2/master/crates/objc2/Cargo.toml>.
- **S40** wayland-client MIT manifest: <https://raw.githubusercontent.com/Smithay/wayland-rs/master/wayland-client/Cargo.toml>.
- **S41** gstreamer-rs licence section: <https://docs.rs/crate/gstreamer/latest>.
- **S42** FFmpeg legal/configuration guidance: <https://www.ffmpeg.org/legal.html>; upstream licence: <https://github.com/FFmpeg/FFmpeg/blob/master/LICENSE.md>.
- **S43** mpv copyright/build/licence conditions: <https://github.com/mpv-player/mpv/blob/master/Copyright>.
- **S44** wgpu workspace licence: <https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/Cargo.toml>.
- **S45** LGPL-2.1 full text: website fetch timed out, but sections 4–6 read locally at `/usr/share/common-licenses/LGPL-2.1`; canonical reference <https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html>. Actual release-binary compliance review remains outstanding.
