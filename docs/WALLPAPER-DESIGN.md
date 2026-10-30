# Live wallpapers: one continuous canvas

Revision 2 · 30 October 2026 · Wave 6E · **decision-ready design, not an implemented feature**.

## What changed since revision 1

Every numbered finding in the [independent review](WALLPAPER-DESIGN-REVIEW.md)
has a disposition below. Spike evidence is retained, not relabelled as production
acceptance. Section numbers identify the replacement contracts.

| Review finding | Resolution or conscious rejection |
|---|---|
| 1. Encoder/codec contradiction | §5 chooses pinned, isolated GStreamer without libav; VP9/libvpx default, optional AV1/libaom. No H.264/HEVC output or GPL encoder; software royalty-free decode available by design on every OS. |
| 2. 4096-wide cap versus wall width | §3/§5 separate global canvas and decoded representation; 4096×2160 is a default admission tier, not a canvas limit. Wide sources need measured opt-in, not implied full-resolution playback. |
| 3. GNOME bridge licence | Ship only with one-mk, independently authored MIT OR Apache-2.0; no extensions.gnome.org listing. Host/dependency legal review still required (§8). |
| 4. Hidden-window/frame callbacks | 6B proved one owned window feeding clipped clones on headless GNOME 50.1. Adopt that instead of the N-window precedent; physical GPU/HiDPI remains a gate (§4). No Hanabi code. |
| 5. Display latency versus clock error | §6 separates clock uncertainty, reported presented-PTS error and filmed display error; predicted present time, calibration and manual nudge. |
| 6. Clock plumbing/topology | §6 daemon timestamps, negotiated datagrams as production target, no claim 6C streams already implement this. Direct paired coordinator links only. |
| 7. Fragile decoder-rate nudges/loops | Reuse 6C controller, implement rate via frame selection, not decoder instant-rate-change; closed GOPs, epoch rebase and explicit hard-cut loop (§5/§6). |
| 8. Hybrid GPU | Integrated GPU first, inspect actual adapter; no silent dGPU activation (§5). |
| 9. Unrealistic estimate | Replace 28–42 with 70–110 engineer-weeks, explicit milestones/reserve and post-M1 re-estimate (§10). |
| 10. Plasma understated | Excluded from release one by owner; no cheap plugin assumption or decoder inside plasmashell. |
| 11. Gallery scarcity/rights | Tens of curated compatible items, not thousands of loops; per-item evidence, rechecks, takedown/contact (§8). |
| 12. URL/peer/security gaps | Concrete SSRF/resource bounds, IP pinning, no proxy inheritance, sandbox first, signed rollback/freeze-resistant catalogue (§7/§8). |
| 13. Missing policies | Reduced motion/photosensitivity, remote sessions/user switching/lid, SDR/HDR/VRR/capture limits, no telemetry, bounded cache (§5/§7/§9). |
| 14. WorkerW fragility | One spanning desktop window; actual hierarchy/build validation and fail-closed behavior (§4). |
| 15. Packaging traps | No libav/libmpv/host autoplugging/whole-plugin-tree bundles; dynamic replaceable LGPL runtime, artifact audits and macOS signing gate (§8). |
| 16. Overconfidence | GO WITH LIMITS is not a shipping verdict; hidden-window physical qualification remains a stop gate (§2/§10). |
| 17. AVPlayer/WebM assumption | Reject native-container assumption; explicit decoder and Metal presenter. No AVPlayer timing dependency (§4/§5). |
| 18. Physical DPI | Logical continuity now, manual logical bezel gaps; physical-mm calibration later, visible mismatch warning (§3). |
| 19. Compositor power | Measure/budget Shell/DWM/WindowServer too; hide/destroy surfaces while paused, no speculative watts claim (§7). |
| 20. Bridge updates | Exact handshake, tested Shell versions, matched component activation/rollback, static on either-direction mismatch (§4/§8). |

| Spike evidence | Revision-2 consequence |
|---|---|
| 6B: one hidden window, two virtual monitors, 59.8–60 fps; Shell 3.1–10.2% of one core | External renderer plus clipped Clutter clones. Python stand-in CPU is not a Rust/video budget. |
| 6B: no JS DMA-BUF import; software frame uploads take 10–15 ms/frame on Shell main thread | Both rejected as live product paths; no frame bytes or native decoder loaded into Shell. |
| 6B: fullscreen does not stop clones; explicit hide does | Bridge enforces visibility, not just a pause request. Multi-window union coverage is new work. |
| 6B: lock/DING extension rebase kills renderer; test client returned in 0.27 s | Treat as teardown/rejoin; real decoder restart latency UNVERIFIED. |
| 6B: hot-plug needs replacement window; adoption 7–13 ms, visible gap unmeasured | New owned window per local layout; allow static transition, no seamless-hot-plug promise. |
| 6B: hang fallback 3068 ms; clean disable and spoof rejection | Keep watchdog/backoff; tighten PID binding/cleanup and unicast product signals (§4). |
| 6B: Overview remains static; GNOME 45–49 not run | Matching poster is opt-in; advertise only individually qualified Shell versions. |
| 6C: tested estimator/controller/64-byte codec and frame selection | Reuse I/O-free `one-mk-wall-sync` in `crates/wall`, not a second clock algorithm. |
| 6C: 1,152 synthetic hour runs; good/bad Wi-Fi P95 16.667 ms, selected max 33.333 ms | Supports further implementation, not a physical 50 ms claim. |
| 6C: persistent one-way delay P95 266.667 ms; more probes do not help | Unobservable asymmetry warning; qualify with filmed scanout, never clock fit alone. |
| 6C: real loopback proxy offset P95 0.052/1.090/3.891/61.116 ms (wired/good/bad/adversarial) | Clock-offset measurement only; no real distributed display gate passed. |
| 6C: selected good/bad models skip 198/138 frames/hour despite no post-start corrective seeks | No invisible-correction or frame-exact promise. |
| 6C: 11520×2160 one-decode/three-crop software throughput 38.81 fps | No software 3×4K60 promise; measured admission and downscale/fps/poster ladder (§5). |
| 6C: Xvfb stitched crops have zero maximum channel error | Spatial evidence only; not atomic physical presentation. |
| 6C: LGPL-labelled libav loads GPL-enabled FFmpeg; patch audit unresolved | Exclude libav/FFmpeg; actual binary dependency/licence/advisory verification gates packaging. H.264/x264 benchmark is evidence only. |

## 1. Executive summary — the owner's decision

**Recommend GO for gated implementation, not a release announcement.** An
optional component, default **OFF**, paints one scene across the existing shared
Layout. Each computer decodes once and displays its slices. Wallpaper failure
must never break KVM, desktop icons or input.

| Release one | Later, not silently promised |
|---|---|
| Linux GNOME Wayland on qualified Shell versions, Windows and macOS on qualified builds/hardware | Plasma, wlroots, other Linux desktops/X11; GNOME 45–49 until individually tested |
| Multi-computer from first playback milestone; explicit receiver consent; reserve absent screens | Automatic leader election, routed sync, physical-DPI calibration, arbitrary gap compression |
| Muted VP9/optional AV1 video, GIF/APNG/animated WebP imports; own files/direct HTTPS | Shaders, web pages, audio, scraper/streaming-site imports, playlists/day schedules |
| Cover/contain/stretch/tile over the whole canvas, logical bezel gaps | HDR matching, genlock/frame lock, seamless-loop or invisible-hot-plug guarantees |
| Commons CC0/public-domain download gallery, tens of audited items, no bundled video | Other sources/licences without new clearance |
| Best-effort ~50 ms P95 on qualified LAN/Wi-Fi, honest quality indicator | Universal Wi-Fi or physical accuracy inferred from clock offset |

**Effort: 70–110 engineer-weeks remaining**, including integration, security,
packaging, independent review and physical QA. Adopt the review's range, not
28–42: narrower Linux scope saves work, but three native adapters, media isolation,
power enforcement and a multi-computer lab remain. Spikes retire architectural
ideas, not production gates. Calendar duration depends on staffing/hardware;
this is not 70–110 elapsed weeks or a date commitment. Re-estimate after M1.

**Top five risks:** (1) private Shell APIs/undocumented WorkerW; (2) VP9/AV1 GPU
limits, hybrid copies and compositor power; (3) asymmetric network/display delay
defeating sync; (4) hostile media and GPU-capable sandbox/licence packaging;
(5) physical mixed-platform qualification unavailable on the one-monitor dev box.

**Owner must test:** Linux↔Windows↔macOS, two physical seams across computers,
one computer with multiple monitors, mixed DPI/rotation/refresh, hybrid iGPU/dGPU,
AC/battery/lock/sleep, real Shell/Explorer/Finder, filmed 30-minute sync and a
24-hour soak (§11). Until then, each platform is **UNVERIFIED on real hardware**.
If a required OS or GNOME fails, hold the full release claim. Shipping a subset
requires an explicit owner scope change, not quiet deferral.

## 2. Evidence and process boundaries

**MEASURED** means only the experiment described in
[6B](spikes/gnome-live-wallpaper-spike.md) or
[6C](spikes/wallpaper-sync-spike.md). **VERIFIED-REPO** means inspected code.
All future contracts/budgets below are **PROPOSED** unless labelled otherwise;
**UNVERIFIED** is a gate. No Rust changes, builds, new benchmarks or desktop
changes belong to this documentation revision.

VERIFIED-REPO: `Rect`/`Desktop` define local logical screens; `Control::Layout`,
status screens/offsets and `motion.rs` carry relative placement. Protocol version
is 2; `MAX_SCREENS` is 64. Wallpaper tightens its scene to 16 outputs/8 devices.
Layout is not already a globally committed scene. Existing paired QUIC and
resumable BLAKE3 transfer are plumbing, not authority to play arbitrary files.

```text
native UI / CLI ── owner-authenticated local IPC ── daemon (keys, consent, QUIC)
                                                      │ bounded wall IPC
GNOME bridge ── bounded D-Bus ── optional one-mk-wall ───┘
   │ owns/spawns display client       │ one decode → N crops
   └─ clipped clones                 └─ restricted media worker
explicit import → bounded downloader → offline normalizer → verified cache
```

Windows/macOS use a per-user supervisor instead of the GNOME launcher. Keep the
existing I/O-free `one-mk-wall-sync` separate from a new presenter package; do
not add GStreamer to the daemon, input backend or timing core. No unauthenticated
listener. Owner-only local sockets/pipes and peer credentials protect control;
inherited one-use handles bind children to the authorized scene. Workers get
read-only media handles/bounded channels, no keys, document access or generic RPC.

## 3. Canvas and committed scene

### Coordinates, fits and slices

For local logical monitor `R(device, screen)` and existing Layout offset
`T(device)`, global `G = T + R`. Use checked signed-64-bit edge arithmetic and
finite transforms. `B` encloses the union of committed rectangles, including
holes/negative origins/reserved offline space. Normalize by `B.min`, never the
primary screen. Do not use pointer gap-clamping for texture lookup.

Example: A `(0,0,1920,1080)`, B `(2020,-120,1280,1024)` gives
`B=(0,-120,3300,1200)`. A samples canvas y=120, B x=2020/y=0; 100 logical units
between screens consume picture space. Additional bezels are explicit
wallpaper-only logical gaps in the Layout overlay, default zero; never change
KVM pointer offsets. No inference of physical size from pixel count/EDID.

Normalize source rotation/sample-aspect ratio first to `Vw×Vh`. Apply one
centre-anchored source-to-canvas transform over `Bw×Bh`:

| Fit | Global mapping |
|---|---|
| Cover (default) | Uniform `max(Bw/Vw,Bh/Vh)`, centred crop |
| Contain | Uniform `min(...)`, centred, chosen opaque padding colour |
| Stretch | Independent x/y scale, intentional distortion warning |
| Tile | One display-source pixel per logical unit; repeat via positive modulo from `B.min`, never reset tile phase per monitor |

For every physical output pixel centre: undo OS output rotation/scale into local
logical coordinates, add `T`, subtract `B.min`, then invert the global fit.
Sample that UV from the same immutable frame/PTS. Identical filter/pixel-centre
and edge conventions prevent independent crop rounding. Retain fractional UVs;
align subsampled chroma access without rounding each monitor seam independently.
Contain uses padding, not edge smearing; tile uses modulo.

Fractional scale changes sampling density, not global logical width. Apply
90/270° rotation exactly once: Layout rectangles are already oriented, physical
buffers need the inverse transform. Mirrors share geometry and do not expand B.
Reject non-mirror overlaps until owner fixes preview. Mixed physical DPI can
visibly change object size: logical continuity is not physical-mm continuity.
Physical calibration later must not reinterpret KVM coordinates. SDR sRGB/
Rec.709 only; HDR matching, VRR and mixed-refresh scanout parity unproven.

### Manifest, consent and membership

Bound scene manifest to 64 KiB, 8 devices/16 outputs, labels/IDs ≤256 UTF-8 bytes
and global edges ±1,000,000 logical units. Fields: schema version, random 128-bit
scene ID, paired coordinator fingerprint, authority generation/revision/layout
hash, ordered member/screen IDs, G/scales/rotations/bezel gaps, fit/background,
media and variant BLAKE3 hashes, rational duration/frame timebase, loop policy,
rate/media-start/future coordinator monotonic epoch. No peer paths or URLs.

Coordinator resolves relative offsets; reject inconsistent cycles, side-only
ambiguous placement, duplicates, overflow/unintended overlap. Preview the frozen
scene in Layout; wallpaper never rewrites KVM placement. Prepare → validate/
verified cache/pre-roll → Ready → Commit future epoch. Start ready members only,
show excluded ones, allow cancellation; preparation times out after 30 s. Do not
claim all screens joined if one was excluded. Commit revisions atomically;
stale generation/revision rejected. Late members join current PTS, not zero.
New media needs local approval unless receiver explicitly authorized this
coordinator's scene changes. Pairing alone is not that permission.

Direct authenticated pairing to coordinator is required. A–B/B–C but no A–C:
choose B or pair A–C. No transitive trust, forwarding or composed clock fits.
No auto-election; explicit takeover requires receiver acceptance and a new
authority generation, invalidating old probes/handles.

Disconnect/sleep/lock/unplug reserves geometry; others retain B/epoch. A visible,
AC-powered disconnected member may free-run its cached slice with stale quality.
“Fit to connected screens” explicitly commits a new B at a future epoch, never
surprise zoom. Added outputs require preview/commit; removed outputs stop locally
immediately. Local hot-plug can replace presentation surfaces without reflowing
the group. No sleeping member holds a group-wide barrier.

## 4. Native presentation and bounded GNOME bridge

### GNOME architecture proved only in an isolated session

6B MEASURED on **headless GNOME 50.1/Mutter 18**: extension spawns fixed installed
`one-mk-wall --present=gnome` with `Meta.WaylandClient.new_subprocess`;
`owns_window` identifies its surface, not title/app-id. Set DESKTOP type before
configure, suppress window-list presence; later stick/lower/hide source actor.
One local spanning window feeds clipped `Clutter.Clone`s above native static
wallpaper in private `Main.layoutManager._backgroundGroup`, below DING/windows.
No normal pinned-below fallback: it stole focus and missed required layering.

**M1a presentation finding (2026-10-30; still STOPPED):** GStreamer 1.28.2
`waylandsink` acknowledges the initial xdg configure, but commits empty root
state before attaching its first root buffer. On Mutter 50.1 that consumes the
pending acknowledgement without establishing the window's last acknowledged
configuration; first content then triggers Mutter's workaround warning. A
NORMAL-window control reproduces it too: DESKTOP conversion is not required
for this particular warning. Do not describe this as an absent wire ack, or
as proof that all compositors reject the sequence.

The feature-gated trusted-fixture diagnostic now uses `appsink` plus a Rust
`wayland-client` xdg toplevel: initial empty commit, receive configure, then
acknowledge once immediately before attaching/committing the first nonempty
buffer, without an intervening empty commit. Later serials belong only to the
current surface; released buffers and frame callbacks bound presentation.
The extension still adopts by connection ownership, sets DESKTOP before map,
and clones one decoder's output per monitor. Empty input regions avoid source
window input. This removes the initial-content warning in isolated runs,
**not** the complete lifecycle warning gate: stack/allocation warnings and
retained managed windows remain. `gtk4paintablesink` is unavailable in this
lab and has not been runtime-qualified. See the [resolution and continued
stop](spikes/wallpaper-m1-stop.md) for primary evidence and the next experiment.
CPU shared-memory copies, fixed-fixture scaling, broadcast spike signals and
manual test restarts are not the production GPU/HiDPI/bounded bridge design.
No live-desktop deployment or shipping presenter is authorized by this result.

No decoder/native module in Shell. No JS DMA-BUF import (NO-GO in 6B); no live
software frame uploads (10–15 ms/frame on Shell's main thread). Wayland window
buffers allow Mutter to handle textures, but actual hardware zero-copy/HiDPI/
hybrid behavior remains UNVERIFIED. Initially qualify GNOME 50; do not claim
45–49 from API reading. Keep Shell version validation on, declare only tested
versions, omit `unlock-dialog`. Test unsafe-mode harness never ships.
Explicit normal extension installation/enabling; explain possible logout/login
(not verified by fresh-session spike). Never restart the owner's live Shell.

### Product D-Bus v1 (derived from 6B, fully bounded)

Name **`org.onemk.WallpaperBridge1`**, object
**`/org/onemk/WallpaperBridge1`**, interface **`org.onemk.WallpaperBridge1`**;
extension owns without `ALLOW_REPLACEMENT`. No properties.

```text
Hello(u protocol) -> (u protocol, u layout_serial, a(iiiid) monitors)
SetState(u state) -> ()
Layout(u layout_serial, a(iiiid) monitors)          [unicast signal]
PauseHint(u reasons)                               [unicast signal]
```

`u` uint32, `i` int32, `d` finite double. Protocol exactly 1; unknown version
returns `org.onemk.WallpaperBridge1.NotSupported`. Monitor tuples `(x,y,w,h,scale)`
are local logical rectangles, canonical Layout order, ≤16, positive w/h, scale
0.5–4. Checked edges; local physical buffer extent ≤16384/side and ≤64 million
pixels, or smaller probed GPU limit. Empty means no outputs/no surface. Payload
≤4 KiB. Rotation/stable IDs come from authenticated daemon manifest, not new
unbounded D-Bus fields; cross-check local geometry before adoption. Serial is
nonzero local layout generation, separate from scene revision; wrap restarts
bridge session. Keep current/latest pending only; coalesce events for ≤100 ms.

State 0 starting, 1 playing, 2 paused, 3 failed. Unknown enums/bits/invalid geometry
return `org.onemk.WallpaperBridge1.InvalidArgs`; unauthorized sender gets
`org.freedesktop.DBus.Error.AccessDenied`. Reason mask values: 1 covered (all local
outputs), 2 locked, 4 overview, 8 display-off, other bits zero. Mask zero permits
resume, never overrides local battery/manual/session restrictions. Hide covered
clones per monitor; all-covered sets the bit and stops decoder. Hello binds and
returns layout, then immediately unicast current PauseHint; start hidden until
that snapshot arrives. No FrameReady, pixels, media selection, paths, URLs,
code, spawn parameters or generic command channel.

Authentication/resource rules:

1. Fixed argv only, while locally authorized/enabled, unlocked, AC-powered and
   potentially visible. Desired state comes from owner-authenticated daemon IPC;
   cold start reads once, dormant extension otherwise uses events, no polling.
2. First Hello verifies current UID/exact spawned PID using bus credentials;
   bind that unique bus name. Later calls compare binding without repeated PID
   lookups. Clear PID/binding on exit/owner loss/disable and before spawn: no
   PID-reuse window. At most one pending credential lookup, four/second.
3. Signals go only to bound unique renderer name, not spike's broadcasts.
   Renderer checks bridge owner PID equals `org.gnome.Shell` owner PID, pins
   unique owner; owner change forces Hello again.
4. Only owned correctly sized windows adopted. One current + one pending for
   ≤2 s replacement; only one painted. Unexpected extra owned windows trigger
   renderer teardown, not ordinary visible orphan windows. No title matching.
5. SetState ≤10/second, duplicate states coalesced; replacement creation
   ≤2/second. Rate accounting only on events, no idle tick. Arbitrary same-user
   D-Bus saturation cannot be fully prevented here; do not claim it can.

### Lifecycle, pause/resume and recovery

Targets below are PROPOSED unless marked MEASURED:

- Start hidden; Hello deadline 2 s, first healthy frame 5 s; otherwise static
  and bounded restart. Reveal only after playing **and** fresh surface damage,
  not a self-reported state alone.
- Damage watchdog 1 Hz only while adopted animation expects progress; no damage
  for 3 s hides clones and calls `check_alive` (Mutter ping timeout unverified).
  MEASURED hang fallback 3068 ms; target ≤4 s including tick quantization.
  Known still/long animation frame deadlines suppress false stale detection.
- Failed-state repaint re-probe at 5/10/20/40/60 s, at most five attempts; expose
  probe clone ≤250 ms to restore frame callbacks. Then stop all failure timers.
  Spawn retry 1/2/4 s, ≤3 automatic starts in rolling 60 s, then static + doctor
  Retry. Caps prevent no-clone/no-frame-callback recovery deadlock and storms.
- Pause hides clones first, sends reasons, stops renderer/decoder ≤1 s
  cooperatively. SIGTERM then SIGKILL after 2 s if needed, reap ≤3 s. Cancel
  watchdog/re-probe immediately. No paused animation child/timer. Resume fresh
  sync/pre-roll at current group PTS, never hidden catch-up decode.
- Lock/DING/earlier-extension disable can rebase our extension: invalidate old
  callback generation/bus binding, disconnect signals, remove clones/name,
  stop child. Re-enable hidden; if locked remain dormant. Unlock rechecks all
  reasons, spawns once, re-handshakes and rejoins. Do not reuse old windows/maps.
  0.27 s was Python stand-in recovery; real renderer UNVERIFIED, target ≤5 s
  once prerequisites are ready. Repeated cascades use same start cap; no
  overlapping children or battery/manual pause override.
- Hot-plug/mode/scale/rotation hides affected clones, emits Layout. DESKTOP
  window resize failed in 6B: create new owned toplevel at new local canvas
  extent, validate/adopt at redraw, close old. Deadline 2 s then static on
  affected outputs; group epoch unchanged. 7–13 ms adoption was measured,
  visible gap was not; no seamless claim.
- Overview stays native static; hide/pause instead of private per-workspace
  clone integration. “Use matching still picture in Overview” is **opt-in**:
  save GNOME light/dark picture URI/style, install generated poster only after
  consent. Restore only if still ours, never clobber newer owner background.
  Explain OS may reuse it on lock screen; no secure-lock animation/hook.

Cleanup is idempotent/destroy-aware (`connectObject`/`disconnectObject` pattern).
Native static wallpaper always remains below; no focus/input grab on any failure.

### Windows/macOS — proposed and UNVERIFIED

| OS | Chosen renderer approach | Release qualification |
|---|---|---|
| Windows | One input-passive, nonactivating per-monitor-DPI-v2 D3D11/DirectComposition window spanning local virtual desktop; N viewports sample one texture under icons in validated WorkerW hierarchy | Progman/SHELLDLL_DefView/WorkerW and `0x052C` are not a supported wallpaper contract. Test actual Windows 11 24H2, 25H2, 26H1/builds individually; no inherited pass. Explorer restart/virtual desktops/icons/focus/device loss must pass. Fail static, never hide/kill Explorer or recreate endlessly. |
| macOS | Non-key mouse-ignoring desktop-level NSWindow per NSScreen below Finder icons, stationary/all-Spaces behavior; one decode shares CVPixelBuffer/IOSurface-backed Metal crops | Intel/Apple silicon, Finder/Spaces/Mission Control/fullscreen/Stage Manager/Show Desktop, hot-plug, signed sandboxed helpers. Collection flags are not proof. No AVPlayer/WebM assumption or lock/saver replacement. |

Cross-GPU sharing/copy costs unproven. Never silently create N decoders: use one
compatible adapter with disclosed copies, downshift or poster. Completion/frame
callbacks are not panel scanout timestamps on any OS.

## 5. Decoder, normalization and resource ladder

**Decision: pinned dynamically linked allow-listed GStreamer without libav or
FFmpeg on all OSes**, OS-specific GPU presenters. Cost: optional media payload;
benefit: one WebM/timing/normalization contract and no native-container gamble.
Reject pinned LGPL-only FFmpeg for release one: possible to audit, but broader
parser/configuration surface and another compliance system without demonstrated
need. No shipped mpv/libmpv/mpvpaper fallback.

Delivery default **WebM VP9 profile 0, 8-bit 4:2:0 SDR, 30 fps**; optional AV1
same canonical duration/timebase. Exact libvpx/libaom permissive encoder builds
audited. “Royalty-free” is codec selection policy, not universal patent warranty.

| OS | Integrated-GPU path to qualify (UNVERIFIED) | Bounded fallback to implement/test |
|---|---|---|
| Linux | Explicit VA/VA-API VP9/AV1 factory on selected iGPU render node → GL/Wayland window; no nvcodec/dGPU autoselection | Explicit vpx/aom software decode → bounded upload |
| Windows | Explicit GStreamer D3D11 VP9/AV1 hardware session on low-power DXGI adapter, verify textures/copies | vpx/aom software → D3D upload; no paid codec extension requirement |
| macOS | VideoToolbox AV1 only where successful hardware session/throughput is proved, low-power device preference; VP9 hardware not assumed | WebM demux and vpx/aom software → Metal upload; no AVFoundation WebM claim |

Use explicit factories/caps, no `playbin`/decodebin host autoplugging. Exact
plugin names are build records, not assumed universal. Reject hardware routes
needing forbidden libraries. Log actual adapter, driver, hardware flag, copies,
codec/profile and reason. Probe a short local clip only after enable/import;
no disabled GPU polling. No iGPU: report it, use bounded software/poster or
require explicit dGPU choice. Decode and rendering may still wake a dGPU on
externally wired outputs; measure/report rather than claim pinning solves it.

One decoder/device/representation → immutable bounded pool → N crops/outputs.
≤3 ready frames; reference surfaces separately capped by negotiated codec
requirements. Slow output drops old frames, never blocks others or KVM.
No giant cross-computer RGBA canvas. 6C one-decode/three-crop 4K CPU/RSS:
81.83% of one core/161.76 MiB versus 262.17%/336.42 MiB for three decoders.
True three-4K **11520×2160**: one decode/three crops **38.81 fps**, 255.83% core,
376.26 MiB RSS. Short software H.264/fakesink run, NOT VP9/AV1, sustained complex
video, GPU memory or actual presentation. Even 30 fps requires qualification.

### Source detail versus canvas and fallback

Three 1080p screens span 5760 source pixels, three 4K 11520; a 4096-wide source
upscales ~1.4×/~2.8×. Preview shows source detail versus logical canvas/physical
output demand. Default representation ≤4096×2160 (8,847,360 pixels). Opt-in wide
tier ≤16384/side, ≤32 million decoded pixels, subject to smaller driver/codec/
sandbox caps. 11520×2160 needs wide opt-in. Canvas itself can be much larger.

Prefer one common full-scene representation; slower members may use smaller
full-scene variants with same fit/duration/frame IDs/epoch, not different
geometry. Pre-sliced per-device encodings deferred; not an invisible escape
from one-decode requirement or canvas budget.

After sandbox gate, preflight 10 s including complex interval: ≥1.25× target
throughput, ≤1% late frames, stable memory, no thermal/power violation. Proposed
gate, not spike result. Start iGPU 30 fps, then:

1. Smaller approved derivative: width 4096 → 2560 → 1920, preserve aspect/epoch;
   downscale wider sources into that ladder.
2. Actual **normalized 15 fps file**, optionally combined with downscale; merely
   presenting half the frames does not halve decoding.
3. Still poster if bounded software tier fails or “Hardware only” selected.
   Never silently start dGPU/unbounded software fallback.

Three bad 10 s runtime windows downshift; evaluate only while playing. No
automatic upshift until owner retry/next enable, no benchmark oscillation.
Local tier changes announce blur/power reason, not a group slowdown.

### Accepted imports, image timeline and loops

Initial whitelist: VP9/AV1 WebM and GIF/APNG/animated WebP through audited,
explicit Rust decoder features. Reject H.264/HEVC/VP8/Theora/other MP4/MKV inputs
with “convert first” guidance. Own-file support is not “any codec”; gallery
curation requires compatible sources. New legacy import decoder is separate
security/licence scope, not an automatic GPL/system-FFmpeg fallback.

Offline normalize VFR/images to shared rational timeline. Correct GIF/APNG/WebP
disposal/background/previous, partial frames/alpha/finite loops; clamp delays
<10 ms to 10 ms, preserve others on 1 ms ticks, flatten against chosen background.
Same conversion/frame table/hash distributed to all peers, not OS-specific delay
heuristics. Encode images once to VP9; preserve originals. Finite animation ends
on final frame/poster unless “Repeat forever” explicitly chosen.

Closed GOPs ≤1 s, no audio/subtitles/attachments/external streams. Default loop
**hard cut**, labelled honestly: most Commons clips are not loops. Seamless
crossfade/ping-pong later as offline new-hash/new-duration preparation; no second
hidden decoder per loop. No web pages/shaders as a shortcut.

## 6. Synchronization and quality semantics

### Spike estimator/controller as the basis

Reuse `Estimator`, `Timeline`, `Controller`, `frame_for_target`, codec/tests in
`one-mk-wall-sync`. Four timestamps give coordinator-minus-local offset
`((t2-t1)+(t3-t4))/2`, RTT `(t4-t1)-(t3-t2)`. Lowest-RTT half (≥3), median/MAD
outlier rejection, 32 anchors, robust slopes ≥10 s apart. Defaults: eight
exchanges 20 ms apart/5 s while active, fit ±500 ppm, drift slew ≤20 ppm and
offset slew ≤5 ms/burst after acquisition. Drop first post-idle exchange, then
collect eight valid samples; bounded 64 probes/exchange age ≤2 s. Initial retry
after 1 s is proposed hybrid policy, not 6C-measured. Start only after fit ready/
pre-roll at future epoch ≥500 ms ahead and ≥2× preparation latency +100 ms.

Daemon stamps t1/t4 dispatch/arrival, coordinator daemon t2/t3 receive/transmit,
not renderer/UI dequeue times. Negotiate `wall_scene_v1`, `wall_cache_v1`,
`wall_clock_v1` first; unsupported old v2 peer gets no wallpaper messages and
keeps ordinary KVM. Production clock target: authenticated QUIC datagrams,
separate from reliable bulk streams. **6C used two example processes and
bidirectional streams**, not production daemons/datagrams, revocation or
input-priority dispatch. Those gates remain. No GstNetClientClock/GstPtpClock:
they don't provide this paired QUIC consent/generation/presentation contract;
do not add another LAN clock service or assume hardware PTP.

Exact experimental envelope basis, not production tag allocation:

| Bytes [start,end) | Little-endian field |
|---|---|
| 0–4 | ASCII `WCLK` |
| 4 / 5 / 6–8 | version 1 / kind 1 request or 2 reply / reserved zero |
| 8–16 / 16–24 | u64 generation / sequence |
| 24–32 / 32–40 / 40–48 / 48–56 | f64 t1 / t2 / t3 / t4 monotonic seconds |
| 56–64 | u64 connection-local session handle bound to complete authorized manifest |

Exactly 64 bytes, no trailing data. Request t2/t3/t4 zero; reply t3≥t2, t4 zero,
requester stamps arrival. Reject magic/version/kind/reserved mismatch, nonfinite/
outside 0..1e9 seconds, invalid ordering, offset >one day, stale/unmatched
session/generation/sequence/t1. Deduplicate/replay-fence before estimator;
≤64 pending probes/member, 32 anchors, 256 frame metadata entries. Errors never
partially mutate state. No URLs/paths/media payload. Capability-gate new dispatch
before parsing, no unannounced existing-v2-tag changes. Clock traffic ≤20
messages/s/member including replies, bulk initially ≤10% measured link goodput,
one transfer/peer. Input priority must be tested: stream separation is not QoS.

Pausing the coordinator locally must not leave a hidden timing worker alive.
Before teardown, send one authenticated Suspended notice; followers stop clock
probes to it and free-run their committed slices with stale quality. No forwarded
or automatically elected clock authority. If the notice is lost, followers stop
probing after 30 s without valid replies and wait for existing connection/Ready
events. Coordinator resume announces Ready and requires fresh maps/pre-roll;
remote visible animation does not override local lock/battery/coverage rules.
No wallpaper replies or periodic clock work on a locally paused coordinator.

### Presentation and controller policy

Check every 100 ms only while playing; predict presentation at
`now + calibrated display latency`, map into coordinator timeline, select latest
eligible **unwrapped** PTS. Repeat/drop, not decoder instant-rate-change.
6C controller defaults: 8 ms enter/3 ms exit hysteresis, absolute ±0.5% rate,
seek near loop boundary >80 ms, hard resync >250 ms or gap >2 s, corrective
seek/resync cooldown 5 s except startup/interruption. Reset rate to 1 after
pre-roll/seek. Skips can be visible despite no corrective seeks.

Group rate change commits future epoch with media-start equal to old unwrapped
PTS at that epoch, then new slope; no discontinuity from rebasing. Sleep,
backwards clock/generation change or accepted takeover resets maps/controller;
fresh probes/pre-roll at current PTS. UTC/DST doesn't alter active playback.

Auto latency estimates use compositor/presentation or native completion feedback,
not decoder dequeue timestamps. They do not see panel/scanout delay. Manual
per-output nudge ±250 ms, 1 ms steps, default zero; positive delays this output
(choose earlier target PTS). Calibrate on filmed hardware; missing presentation
feedback explicitly weakens confidence, not “zero latency”.

### Quality indicator

Three metrics, never conflated: clock-fit uncertainty/age, reported pairwise
presented-PTS error, filmed physical display error. Use largest pairwise circular
error among active members, not pooled average. Bounded 60 s window/10 Hz
reports ≤600/member. Separate startup, warm statistics after 10 s. Active
renderer publishes; closed UI adds no polling or measurement work.

| Plain-language label + shape | Meaning |
|---|---|
| Measuring | Less than 30 s warm reports, no feedback or reacquiring; no reassuring physical number |
| Good estimate | Valid feedback, accepted burst ≤15 s old, estimated clock uncertainty ≤10 ms, reported P95 ≤50 ms/P99 ≤100 ms; explicitly “estimate; display timing not verified” |
| Degraded | Threshold exceeded, late frames, poster member or burst age >15 s; explain wired link/lower detail/nudge remedy |
| Out of sync / offline | >250 ms reported mismatch or no valid clock >30 s; cached local free-run allowed, never good badge |
| Paused / off | Reasons/scope; no measurements or heartbeat |

Persistent one-way delay is indistinguishable from offset without independent
assumptions. It can fool confidence and even “Good estimate”; always disclose
unverified physical accuracy. Physical lab result is separate/datable, not a
permanent guarantee badge. Higher probe frequency cannot cure asymmetry.

6C good/bad synthetic Wi-Fi P95 16.667 ms, selected max 33.333 ms; adversarial
P95 266.667 ms, no convergence. Loopback proxy offset P95
0.052/1.090/3.891/61.116 ms isn't display accuracy. Selected good/bad runs
skipped 198/138 frames/hour. Target qualified physical LAN/Wi-Fi pairwise display
P95 ≤50 ms/P99 ≤100 ms over ≥30 minutes after 10 s warmup; warm loopback
presented-PTS P95 ≤20 ms still unproved. Publish failures; no frame-exact,
imperceptible-seam or arbitrary-Wi-Fi promise.

## 7. Threat model, limits and zero-idle rule

| Threat/boundary | Required mitigation and test |
|---|---|
| Malicious paired peer | Purpose/coordinator consent, authenticated bounded manifest, cache handles only, replay fencing. No remote generic local RPC, paths/URLs/execute. Forget/revoke closes streams/invalidates pending handles; no remote erasure of already authorized local cached files. |
| Hostile video/image, even normalized by coordinator | Every peer validates/decodes in its own sandbox; hashes prove integrity, not safety. Whitelisted parsers, allocation/time caps, fuzzing and exact patched dependencies. Hardware decode doesn't sandbox firmware/drivers. |
| Hostile URL/catalogue | Separate constrained downloader, no network in media worker; SSRF/signed metadata controls §8 plus schema/resource validation even if signed. |
| Compromised renderer → extension | Bounded state/geometry only, owned/capped surfaces, no code/media bytes in Shell; spoof/flood/churn/hang tests. Can spoof a picture below windows, never trusted prompts/input capture. |
| Same-user takeover/other session | Owner ACLs/credentials and session-local runtime paths; user may replace per-user extension/binary, not a fully solved hostile-user boundary. No cross-session control. |
| Worker → OS/files/network/GPU | Linux namespaces/seccomp with only required render node/display socket; Windows AppContainer or equivalent restricted-token/job capabilities; macOS signed sandboxed helper. Prove denied network/keys/documents and bounded GPU IPC on each OS before decoding. Ordinary process isn't a sandbox; failure blocks adapter animation. |

Admission defaults: source/download ≤512 MiB, duration ≤10 min, source ≤60 fps,
≤36,000 source frames, normalized ≤30 fps, composited-animation working set
≤256 MiB, 2 queued imports/one normalization worker, ≤10 min worker wall time,
≤1 GiB resident worker ceiling including ≤256 MiB metadata/decoder auxiliaries.
Check actual frames/decoded dimensions, not declared duration; reject changes/
bombs before allocation. No MKV attachments/extra video tracks/subtitles/audio/
external references. Output ≤512 MiB; wide opt-in doesn't waive memory caps.
Stream image conversion, never retain every frame. Cancel kills/reaps/removes
partials and preserves previous scene.

Cache default 2 GiB: LRU completed unreferenced media, never active scene/poster/
provenance. Reserve space first; active set fills quota → actionable refusal.
Orphan partials >24 h cleaned at next activation, not off-state timer. Private
content-addressed paths, no symlink/traversal; atomic rename after full BLAKE3
verification even for resumed files.

### Power and per-state budget

Off: no renderer/import/decoder, media libraries in daemon, wallpaper heartbeat,
polling or watchdog. Dormant optional GNOME extension retains passive listeners
only. Imports require Enable on and an explicit user operation, never background
autoplay; enabling the component does not itself start playback. Lock/battery/
inactive session cancels and reaps import workers, retaining only resumable disk
state, not a resident paused parser.

Pause reasons ORed: manual, covered, locked, display-off, battery, low-power,
reduced-motion, remote-session, user-inactive, gaming. Clear **all** applicable
reasons before resume. Battery always pauses release-one animation; no override.
Low-power/reduced-motion defaults poster. Hidden UI preview stops too.

| State | Wallpaper-attributable steady budget / transition target |
|---|---|
| Off | 0 children/timers/packets/frames, 0 periodic daemon wakeups; core RSS unchanged except bounded unloaded config |
| All local outputs covered / locked / battery / display off | Hide/destroy surfaces, stop children; 0 decode/present/probe/watchdog work after ≤1 s cooperative/≤3 s forced cleanup. Passive events only. |
| Some local outputs visible | Hidden outputs don't repaint; one decoder for visible slices, no per-hidden-output frame loop |
| Active admitted 1080p30 | Proposed presenter+decoder CPU ≤5% of one core/RSS ≤300 MiB; compositor delta separately targeted ≤10% core |
| Active 4K30 / wide opt-in | Proposed CPU ≤10% / ≤25% core; RSS ≤512 / ≤768 MiB; GPU allocations ≤512 MiB / ≤1 GiB within driver caps; exceed → downshift/poster |
| Failed | Native static, bounded §4 retries then no failure timer, explicit Retry |
| Explicit import | One limited offline worker, cancelled/reaped on battery/lock/session-inactive; no autoplay |

Budgets are targets, not spike measurements. Measure Shell/DWM/WindowServer,
VRAM/power/dGPU residency too; stopping decoder alone doesn't stop compositor
redraw. Compare hidden/destroyed outputs to native-static baseline. Ten-minute
off/paused traces must show no periodic wallpaper work; bounded event handling
is not a promise of literally zero instructions. If coverage pause cannot be
enforced, adapter stays poster-only, not a silent power-rule exception.

GNOME must compute union of opaque windows per output/workspace on compositor
events (6B tested only maximized/fullscreen). Windows window/power/session events
plus fullscreen suppression including `SHQueryUserNotificationState`; macOS
workspace/display events and, if needed, explicit Accessibility consent for
global coverage. Complete macOS coverage without consent UNVERIFIED: recommend
poster-only until proved, no screenshot polling. Generic Wayland coverage not
assumed portable.

Observe GNOME enable-animations, Windows client-area-animation preference,
macOS reduce-motion; first-use photosensitivity warning. Remote desktop
(Windows SM_REMOTESESSION included), fast user switching/inactive/secure desktop
pause. Closed lid with external screen follows actual output/power state, not
lid alone. No sleep inhibition. Native screenshots/recordings can include the
wallpaper; no capture-exclusion promise or covert capture for coverage.
No telemetry; gallery/download discloses IP/User-Agent to host, explained before
fetch; no off-state catalogue refresh.

## 8. Licensing, packaging, URLs and gallery

### Concrete shipping stack and release gates

Original code/bridge MIT OR Apache-2.0; no Muro PolyForm Shield, Aura GPL-3.0,
Hanabi GPL-3.0 code, translations, close implementation copies, assets/catalogue.
Only independently authored integration from platform API descriptions. Bundled
extension only, not extensions.gnome.org. Permissive MIT option doesn't relicense
GPL Shell/dependencies or replace legal review.

Optional package: pinned patched GStreamer core/needed plugins, WebM parser/
demux, vpx/aom encode/decode, selected OS GPU factories, converters/presenters,
audited image dependencies. **Exclude libav plugin, libavcodec, ffmpeg binary,
x264/x265, libmpv, broad gdkpixbuf autoloading, network/playlist plugins and whole
system plugin trees.** Spike x264/H.264 fixtures never enter release artifacts.

Private audited plugin registry/scanner/search paths; clear inherited
`GST_PLUGIN_PATH`, system plugin paths and library-loader overrides for normal
launches. No host decoder discovery (Debian system FFmpeg may be GPL), including
AppImage. Explicit documented compatible LGPL replacement mode is separate
from silent host autoplugging. Dynamic LGPL libraries retain notices/licences,
source/modifications and replacement/debugging rights; binary isn't “all MIT”.

macOS media-helper signing/hardened runtime must allow demonstrable compatible
modified-library replacement. If necessary narrow `disable-library-validation`
to sandboxed media helper only, never daemon/UI; document local re-signing/
replacement and review legally. No App Store promise. If signing/notarization/
replaceability can't coexist, block that build, not silently static-link LGPL.

Extend `.github/workflows/release.yml` and packaging scripts in M1/M5:

1. Pin revisions/build flags/patch provenance/toolchains and plugin inventory.
   Build controlled optional package, keep core/headless payload media-free.
2. SPDX/CycloneDX SBOM includes Rust/native shared libraries/plugins/codec
   sources/build tools; ship notices and corresponding library source route.
3. Inspect every plugin's licence metadata **and recursive ELF/PE/Mach-O
   dependencies**, plus actual loaded-library inventory in clean-machine decode.
   Fail unknown/GPL/AGPL/nonfree media or libav/ffmpeg; wrapper licence is not
   sufficient (6C). Negative test inserts forbidden transitive/plugin dependency.
4. Scan exact advisories/backports; unresolved relevant parser/decoder issues
   block release. Installed 1.28.2 is not clearance: 6C's parser/image advisory
   audit remains unresolved for the chosen binaries; re-audit during build.
5. Prove modified compatible LGPL replacement on every OS; archive inventory,
   build manifest, SBOM/licences/source and scan results beside signatures.
6. Clean OS install/update/rollback/uninstall, optional payload size measured;
   no daemon media dependency or ambient host-library leakage.

Activate bridge/presenter as matched signed v1 set with previous pair retained.
Older/newer incompatible bridge → Hello refusal/static/doctor, never best guess.
Partial update aborts/rolls back pair; normal reload/logout if needed, no Shell
validation bypass. Doctor reports exact Shell/bridge/presenter/protocol and fix.

### URL import safety

Direct media only on explicit owner request. Peers distribute verified bytes/
cache IDs, never ask receivers to fetch URLs. Downloader has bounded network
but no keys/documents; normalizer/renderer no network:

- HTTPS port 443 only; no credentials/fragments, file/data/ftp/playlists,
  ambiguous numeric IP encodings or IPv6 zone identifiers.
- Resolve/check all A/AAAA answers; reject private/loopback/link-local/
  multicast/unspecified/reserved/cloud-metadata, IPv4-mapped IPv6, internal names.
  Pin validated public address for actual connection, retain original hostname
  for TLS/SNI/certificate checks; verify connected address, no re-resolution.
- ≤3 redirects; revalidate/resolve/pin every target; forbid HTTPS→HTTP,
  proxy/PAC/environment inheritance, cookies or ambient auth.
- Connect 10 s, idle read 15 s, total 5 min, streamed ≤512 MiB even absent/lying
  Content-Length. No HTTP decompression/archive expansion; bounded ≤64 KiB
  metadata, sanitize names, MIME/magic/allowed parser validation.
- Private partial file/hash; close downloader before offline probe/normalize.
  Atomic cancel. Test rebinding, mixed public/private DNS, redirect to metadata,
  mapped IPv6 and traversal. No yt-dlp/browser scraper fallback for web pages.

### Gallery and rights operations

Retain reviewed verdicts: Pexels/Pixabay/Mixkit/Coverr **not approved** feeds;
NASA individually cleared later only; Internet Archive terms/access unverified,
not approved. No new terms audit in revision 2; recheck before implementation.

Commons CC0/jurisdiction-appropriate public domain only, individually reviewed
compatible VP9/AV1 sources. Per item: creator (not assumed uploader), licence/
version/link, source, rights evidence/date, original factual title, credit/
adaptation text, byte size/duration/hash. Prefer scenery without people/logos;
tags aren't Foundation warranties or trademark/publicity clearance. No actual
item is cleared by this document. Expect **tens**, few true loops; no BY/SA/NC/
ND/GFDL expansion just to fill it. Empty unavailable gallery is honest, but
doesn't satisfy promised release-one gallery criterion.

Host bounded signed index/provenance/tombstones only, **never videos** or bundled
previews. Download source bytes only on click, not continuous hotlink playback.
Identify Wikimedia/one-mk and contact in User-Agent, cache metadata/honour
throttling; no white-label provider hiding or mass crawler. Native UI shows
creator/licence/source/cost before fetch; preserve provenance beside cache and
explain transcoding. No copied description/database prose. Previews generated
locally after authorized download or separately cleared on-demand thumbnail.

Use reviewed TUF-style implementation: offline 2-of-3 root keys, delegated online
catalogue signing, old/new threshold-signed root rotation, timestamp/snapshot/
targets consistency, expiry and stored highest versions. Index ≤1 MiB/1,000
entries, metadata depth ≤4. Reject rollback/freeze/expired/hash mismatch;
expiry disables new downloads, not verified own files. Bad local clock requires
owner correction, not bypass. No home-grown signature-only substitute.

Maintainers recheck deletion/licence before each catalogue publication and at
least every 30 days, not each user's idle daemon. Publish contact/takedown and
counter-notice/appeal policy; legal review of applicable DMCA/DSA duties. Aim
acknowledgment ≤2 business days; credible claim temporarily delists with signed
tombstone. Notify next explicit refresh, no silent original-file deletion or
remote wipe; offline clients cannot learn revocations instantly.

## 9. Settings, CLI/UI and failures

Native Wallpaper surface uses existing Layout overlay, no second placement
editor/web view/new always-on tray. Empty state: “Use a moving picture across
your screens”, Enable default off, own-file/URL/Commons choices. Explain power,
photosensitivity/download privacy/receiver authority before enabling. One clear
primary action, accessible labels/keyboard; errors provide remedy, not raw paths.

| Plain-language setting | Default/behavior |
|---|---|
| Enable live wallpaper / Start at login | Both off; component/extension install is separate consent |
| This computer controls the picture | Explicit coordinator, local receiver approval; Apply shows scope |
| Fit / gaps | Cover / preserve Layout; explicit “Fit to connected screens” commit |
| Quality | Automatic, iGPU, 30 fps ladder; Advanced hardware-only/dGPU/nudge |
| Pause | All applicable reasons shown; coverage/lock/battery/power/accessibility enforced |
| Matching still in Overview | Off, consent and conditional restoration |
| Pause here / Pause group / Stop | Local versus authorized coordinator scope clear; stop/autostart distinction explicit |
| Library / Gallery / Remove cached copy | Rights/source shown; never removes original; no automatic network preview |

Proposed, not implemented CLI:

```text
one-mk wallpaper import <file-or-https-url>
one-mk wallpaper gallery list
one-mk wallpaper gallery download <item-id>
one-mk wallpaper authorize <paired-computer> --scope wallpaper
one-mk wallpaper scene preview <media-id> --fit cover
one-mk wallpaper start <media-id> --scope paired --fit cover
one-mk wallpaper pause --local
one-mk wallpaper pause --group
one-mk wallpaper stop --local
one-mk wallpaper status --json
one-mk wallpaper doctor
```

Exit 0 accepted/success, 2 usage/validation, 3 not-ready/unsupported, 4 refused
authority, 5 operational failure. Async commands return operation ID, status
preparing/playing/failed, never accepted-as-done. Atomic versioned settings and
previous-build migrations/rollback; persist policy/consent/hashes, not clock fits,
URL credentials or keys. Keep last good scene on parse failure, KVM untouched.

Doctor/status: build/OS/tested label, bridge/protocol, actual decoder/GPU/copies/
licence build, source/tier/memory/cache, scene/revision/members, reasons, clock
age/uncertainty and reported-versus-physical error, missing permissions/action.
Never installs bridge/grants Accessibility/plays preview silently.

| Failure | Local recovery | Others |
|---|---|---|
| Stop/off | Hide/terminate/reap; conditionally restore opted-in still | Continue, reserve rectangle |
| Coverage/lock/battery/display off/user switch | Hide immediately, ≤1 s cooperative/≤3 s forced cleanup | No implicit group pause |
| Unlock/wake | New map/burst/pre-roll, static until ready, ≤5 s target after prerequisites | Current-PTS join, never restart group |
| Network/coordinator gone | Visible cached free-run/stale quality; frozen manifest; explicit accepted takeover only | No barrier/reflow/election |
| Slow decode/thermal/hardware failure | Repeat/drop briefly, ladder then poster | Group timeline unchanged |
| Renderer crash/hang | §4 watchdog/restart cap then static + Retry | KVM/icons/input unaffected |
| Hot-plug/DPI/rotation | Remove output, new local GNOME window ≤2 s or static; commit added geometry | Preserve B unless explicit reflow |
| Shell/Explorer/Finder/GPU restart | Invalidate handles, revalidate host; one bounded restart sequence | Static if unsupported, no manipulation loop |
| Version/partial update | Static, rollback matched pair, doctor instructions | Old KVM keeps working |
| Bad media/hash/full cache/cancel | Reject partial, kill worker, preserve previous scene | Ready peers unaffected |
| Revoked authority/hostile command | Refuse without side effects, invalidate purpose/handles, bounded redacted log | No onward fetch/execution |
| Gallery expired/offline/tombstone | No new download, actionable contact/source, no remote deletion | Own-file playback unaffected |

## 10. Milestones, tests and estimate

Every increment distributable behind setting **default OFF**, experimental/
unsupported labels retained; not permission to claim full release. First playback
M2 includes two authenticated “computers”, not a public single-PC-only phase.

| ID / remaining effort | Gated increment | Exit criteria and tests |
|---|---|---|
| 6D-M1 / revised remaining 16–26 engineer-weeks | Optional safe package/import/poster + capability/consent scaffolding; M1a diagnostic is not acceptance | Prove sandbox filesystem/network/GPU constraints on all OSes and LGPL replacement/artifact negative tests. Resolve retained windows/stack warnings and qualify the full real-renderer lifecycle; prototype WorkerW/Mac host. Geometry/fuzz, bombs/cancel, off-state no timers. Stop/re-estimate if isolation/presentation fails. |
| 6D-M2 / 10–16 | GNOME multi-computer local video/images | Two real daemons/private sessions, transfer/rehash/epoch/N crops/one decode, all fits/image timing. Datagrams/old-v2 compatibility, fake clocks/asymmetry/outage/revoke/replay. Warm loopback presented-PTS ≤20 ms P95, crop goldens, kill/focus/input cleanup. Other OSes remain static/unsupported-labelled. |
| 6D-M3 / 16–24 | Windows/macOS + sync quality/power | Physical native layering/focus/Spaces/Explorer matrix, actual VP9/AV1/iGPU paths/ladder, union coverage, calibration, sleep/rebase/hot-plug. Zero off/paused periodic work, budgets including compositor. Compilation alone fails gate. |
| 6D-M4 / 10–16 | URL/gallery/native controls/CLI/doctor | SSRF/limits/metadata rotation/freeze/rollback/takedown tests, ≥10 cleared compatible Commons items for gallery-ready. UI all states/screenshots/accessibility; settings migrations/cancel/failures. |
| 6D-M5 / 14–22 | Qualified release-one optional package | Three-OS physical group, filmed wired/Wi-Fi, 24 h soak, clean install/update/rollback/uninstall/sign/notarize, security/licence and other-model review, independent user-journey QA, exact support matrix/SBOM/owner sign-off. |
| Integration reserve / 8–14 | Cross-milestone fixes/security/dependency churn | Re-run affected gates, not excuse to drop GNOME/sandbox/multi-computer scope |
| **Total / 70–110** | **Staged owner-authorized implementation** | **Re-estimate after M1; no calendar-date commitment** |

Narrower Linux offsets costs, not enough to revive 28–42 weeks: new cross-platform
royalty-free pipeline/sandbox work and lab remain. Plasma/other compositors,
physical calibration and schedules need separate estimate. Hardware access
before M3, not final week. 6B/6C research complete; implementation waits for
owner GO/lab, not imaginary outstanding spike results.

### One-monitor development and quality bar

Use 6B isolated headless Mutter/private bus/runtime/home harness, 1/2/3 virtual
monitors. No inherited owner's socket/live desktop, real identities/config or
installed unsafe test extension. Private Xvfb supports crop checks, not real
RandR wall/desktop qualification. Generate small original clips in ignored
worktree lab storage, not RAM-backed `/tmp`; always reap owned children/files.

- Geometry: negative/gapped/scaled 1/1.25/1.5/2/portrait/mirrored/rejected overlap/
  cycle/huge layouts, bezels/SAR/rotation/all fits/chroma. Stitched crops versus
  CPU reference at fixed PTS; exact nearest pixels, stated GPU filter tolerance.
- Images: disposal/alpha/partial/previous/background/delay/finite loop/truncation,
  late-join same frame; fuzz header and decoded limits, mutation breaks tests.
- Timing: reuse 6C seeds plus independently transformed per-process clocks;
  sleep/backwards/takeover/one-way delay, 0/1/5% loss, reorder/correlated outage,
  decoder stalls/input-bulk contention. Shared host clock isn't distributed proof.
- E2E: two/three real daemons/null input/ephemeral pairing/separate caches/private
  displays; real CLI/import/transfer/commit/status, not mocked file copies.
  Kill -9 at prepare/commit/rename, full disk/revoke/old peer/bad wire/spoofed bus/
  pause-refusing renderer. No stuck keys/buttons/pointer or orphan processes.
- Lifecycle: 100 enable/disable/lock/DING cycles, rapid hot-plug, extra-window
  refusal, long-frame deadlines, no-clone recovery deadlock; no Shell warnings,
  leaks/focus grabs or off-state timers.
- Resource: ten-minute baseline/off/paused traces, CPU/RSS/wakeups/compositor/
  VRAM/power/dGPU residency; actual active queues/counts. Input P95 regression
  ≤1 ms versus controlled loopback baseline under media transfer; record noise
  and physical input results, no silent relaxed threshold.

Apply [FEATURE-PLAN quality bar](FEATURE-PLAN.md#wave-5-chosen-by-the-owner-from-a-checklist-2026-10-30):
failing-first tests/integration, real binaries and failure runs, workspace tests/
clippy ≤4 jobs during implementation, help/exit codes/actionable errors, every
UI empty/error/offline/long-name/narrow/HiDPI/keyboard/reduced-motion/contrast
state with inspected screenshots, on/off measurements, no leaked resources,
property/fuzz/mutation tests, README/help/troubleshooting/doctor, honest tested
labels. Hostile self-review, other-model review, independent user-journey QA are
distinct gates. This docs-only task runs no builds or `cargo fmt` and does not
count as an implementation validation pass.

Reports retain commit/build/OS/Shell/GPU/driver/codec/adapter/copies, fixture/
layout/hash, commands, timestamps/sample window, input baseline, P50/P95/P99/max,
resource data/diffs/logs, passed/failed/skipped and recovery timings. Separate
unit/compiled-only/offscreen/virtual-session/physical E2E; no cross-target clippy
or screenshot certifies native hardware.

## 11. Owner-hardware acceptance script — not run here

After implementation, save work, use consenting disposable test logins and
static/settings backups. Record versions/builds/GPU/driver/runtime. Original
numbered grid/moving-bar VP9/AV1 plus GIF/APNG/WebP; Linux GNOME/Windows/Mac,
≥2 physical seams, ≥1 device with multiple monitors, mixed scale/portrait/
negative placement/refresh and hybrid laptop. Proposed CLI is not yet available.

1. Clean install default off: no children/media library/heartbeat. Explicitly
   install/enable matched GNOME bridge, doctor verifies. Windows 24H2/25H2/26H1
   separately if claimed; Intel and Apple silicon if both claimed. Unsupported
   combination static/actionable, never compile-check masquerading as support.
2. Pair/approve coordinator per receiver, explicit Layout offsets, preview whole
   canvas, import once/check hashes/variants. Verify disjoint slices/grid/holes/
   bezels/four fits and all image types, physical-DPI mismatch disclosed. Test
   5760/11520-wide demand and smaller source/visible quality downgrade.
3. Click/drag/context-menu icons, Alt-Tab/Show Desktop, workspaces/Overview/DING,
   Windows desktops, Mac Spaces/Mission Control/Stage Manager/fullscreen. No
   focus theft/input capture. Poster opt-in only; change static background
   yourself then stop, verify newer owner setting isn't overwritten.
4. Maximize/fullscreen and cover each monitor with several windows; all-covered
   stops children/timers/probes/presentation, uncover one restarts one decoder.
   Lock/battery/power/reduced motion/remote desktop/user switch each pause; clear
   one reason while another remains. Lid/external output and sleep/wake correct;
   no secure-screen animation, current-PTS rejoin.
5. Hot-plug/DPI/rotation/replug; expect static transition, record visible gap.
   Disable earlier GNOME extension/lock repeatedly: cascade cap/no stale actor
   or Shell crash. Disposable Explorer restart, GPU loss/renderer kill/hang:
   fallback ≤4 s, cleanup ≤3 s, ready restart ≤5 s target after prerequisites.
   Real unlock latency measured, never substituted with spike's 0.27 s.
6. Film numbered frames across every seam ≥120 fps for ≥30 min each wired,
   normal Wi-Fi and congested Wi-Fi, after 10 s warmup. Compute largest pairwise
   physical P50/P95/P99/max/error/skips/displacement, camera uncertainty (120 fps
   sampling 8.33 ms). Target P95 ≤50/P99 ≤100 ms; borderline uncertainty needs
   faster capture, not rounding to pass. Compare estimator/nudge to film; inject
   asymmetry and verify warnings/unverified physical accuracy, not guaranteed
   green. Log outliers/startup separately too.
7. Bulk transfer/KVM input under playback: latency baseline/regression. Sleep/
   disconnect coordinator/receiver: reserve crop/epoch, cached free-run/stale,
   no election/reflow. Explicit takeover rejects old authority; late rejoin
   doesn't rewind group. All participants paused → zero wallpaper traffic.
8. Truncated/hostile media/URL redirect/private-IP, full/read-only cache/cancel,
   bad signature/expiry/revoked consent refuse safely. Real cleared Commons
   download shows creator/licence; offline/missing codec actionable, no scraper/
   paid/GPL auto-install fallback.
9. 24 h soak with hourly restart/layout/power events, no growing queues/cache/
   RSS/VRAM; record decoder count/compositor power/dGPU. Upgrade previous
   settings, partial signed update/rollback, stop/uninstall optional package.
   Originals/other wallpaper apps untouched; no leaked input/process/temp file
   or disabled idle regression.

Owner signs exact matrix with failures/skips; independent QA follows journeys
without author help. Missing hardware means **not tested**, not virtual pass.
Required OS/GNOME/sandbox/power/physical-quality failure blocks full release
unless fixed or owner explicitly revises scope/target.

## 12. Open questions for the owner

Already decided: three OSes, GNOME release one/no Plasma, multi-computer,
VP9/AV1/no GPL encoders, iGPU, own/URL/Commons, best-effort sync. Do not reopen
those implicitly. Remaining choices:

| Question | Choices and recommendation |
|---|---|
| Q1. Authorize investment? | **A (recommend):** fund M1 (12–18 engineer-weeks), then re-estimate within 70–110. B: authorize all milestones now with same gates. C: hold, retain evidence only. |
| Q2. GNOME breadth? | **A (recommend):** qualify named GNOME 50 builds first, older versions later. B: add selected 45–49 versions before release with added lab/effort. No unchecked range. |
| Q3. Legacy imports? | **A (recommend):** VP9/AV1 WebM + GIF/APNG/WebP first, “convert first” errors. B: add audited isolated legacy import decoders before release, new estimate, still only VP9/AV1 output/no GPL artifacts. |
| Q4. Native adapter/coverage fails? | **A (recommend):** keep static/experimental and block full release until fixed. B: explicitly narrow release scope, never call subset all three. |
| Q5. Missing computer space? | **A (recommend):** reserve, explicit reflow only. B: auto-reflow opt-in, extra consensus/UX work and visible zoom. |
| Q6. Overview matching? | **A (recommend):** unchanged still until opt-in. B: prompt once in GNOME setup, explain lock-screen reuse. Never silently change. |
| Q7. Physical Wi-Fi target fails? | **A (recommend):** fix/calibrate or restrict qualified configurations, warn/hold required matrix. B: explicitly approve looser measured target and publish rationale. Never frame-exact. |

### Evidence carried forward

[Review](WALLPAPER-DESIGN-REVIEW.md), [GNOME spike/transcripts](spikes/gnome-live-wallpaper-spike.md),
[sync/media spike/transcripts](spikes/wallpaper-sync-spike.md),
[feature plan](FEATURE-PLAN.md), [UI review](DESIGN-REVIEW.md),
[spatial prototype](design/spatial-prototype.html). Revision 1 remains in git
history: its 28–42 week range/native-codec assumptions are superseded, not erased
evidence. Spikes retain primary references/legal observations. Re-audit current
terms/security advisories/exact binaries in M1/M5. This is engineering planning,
not legal clearance or an already verified product architecture.
