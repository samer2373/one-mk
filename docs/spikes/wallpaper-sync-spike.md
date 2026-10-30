# Wallpaper synchronization feasibility spike

Measured 2026-10-30; branch `t3/wallpaper-sync-spike`, based on `main`
`9643605`. Design input: `t3/research-live-wallpaper-design:docs/WALLPAPER-DESIGN.md`,
especially sections 4, 5, 7 and 9. This is research, not an enabled wallpaper feature.

## Verdict first

**The 50 ms P95 target is achievable in the good and bad Wi-Fi simulations,
but is NOT yet verified between physical computers or displays.** Both models
achieved **16.667 ms P95**, including all tested parameter combinations. In the
selected four-computer run their maximum errors were **33.333 ms**. An adversarial
one-way-delay model achieved **266.667 ms P95: a clear failure**, unaffected by
more frequent probes. No timestamp algorithm can distinguish a constant
one-way delay from a clock offset without additional assumptions.

At a seam the owner could see a one- or two-source-frame temporal step, not
necessarily a black flash. At 300 pixels/second, the selected 16.667/33.333 ms
errors mean roughly 5/10 pixels of displaced motion. The adversarial 266.667 ms
error means roughly 80 pixels. These distances are arithmetic illustrations,
not measured physical seam artifacts. Nudging sometimes skips a source frame:
there were **198 good-Wi-Fi and 138 bad-Wi-Fi frame skips** in the selected hour,
despite **zero post-start seek discontinuities**. Do not advertise invisible seams
or frame lock.

Recommended starting policy: **eight probes 20 ms apart, every 5 seconds while
active; 32 burst anchors; 100 ms renderer checks; ±0.5% maximum rate adjustment;
8 ms enter / 3 ms exit hysteresis**. Cap the clock fit at ±500 ppm, change drift
by at most 20 ppm and offset by at most 5 ms per accepted burst. Require readiness
and a future epoch after initial synchronization. The sweep did not establish
that 5 seconds or 32 anchors are uniquely optimal; they are conservative defaults.

Surprises: the LGPL-labeled GStreamer `libav` plugin loads a **GPL-enabled system
FFmpeg build**; three horizontal 4K screens are **11520×2160**, not 7680×2160;
and that true three-4K one-decode/three-crop software pipeline reached only
**38.81 fps** here. One decode is reusable and substantially reduces CPU/RSS,
but does not make arbitrary giant media cheap. Hardware decode was not tested.

## Evidence boundaries

- VERIFIED-CODE: dependency-free, I/O-free Rust library plus tests; existing
  daemon, codec and protocol version unchanged. MIT OR Apache-2.0, version
  inherited from workspace (0.1.13).
- MEASURED-MODEL: 1,152 seeded one-hour simulations, each 216,000 frame samples.
  These are synthetic delay models, not measurements of Wi-Fi radios.
- MEASURED-HOST: real paired QUIC between two processes through a UDP proxy;
  software H.264 decoding; cropped windows on **private Xvfb :187 only**.
- UNVERIFIED: actual cross-host presented PTS, GPU/vsync/scanout, audio behavior,
  GNOME/Windows/macOS wallpaper bridges, decoder sandbox enforcement, battery
  power, thermal steady state, long-form complex-video performance and a complete
  vulnerability/licensing audit. No production quality gate is passed by proxy.

No wallpaper setting, extension, input backend, installation, live desktop
surface, or `~/.config/one-mk` was touched. Tests used ephemeral identities and
loopback sockets. Owned child processes were killed/waited; media scratch was
kept in ignored `target/wall-spike`, not the small RAM-backed `/tmp`. No release
build, fat LTO or `cargo fmt` was run.

## A. Reusable synchronization core

`crates/wall` contains `one-mk-wall-sync`. The core has no networking, system
clock reads/writes, renderer, GStreamer or async runtime dependencies.

`MonotonicClock` injects seconds and a clock generation. `Probe::measurement`
computes coordinator-minus-local offset `((t2-t1)+(t3-t4))/2` and network RTT
`(t4-t1)-(t3-t2)`. `Estimator` fences session/generation, rejects replayed bursts,
deduplicates replies, chooses the lowest-RTT half (at least three samples),
then applies median/MAD outlier rejection. It keeps 8–32 tested anchors, uses
median slopes separated by at least 10 seconds, bounds skew and slews the map.
The first valid offset is unrestricted within the one-day policy bound; later
updates are bounded. Transport adapters must match outstanding sequence and
t1 before submitting a burst; the core cannot authenticate unsolicited data.

`Timeline` maps a committed coordinator epoch/media start into a looping PTS.
`Controller` checks the renderer's own monotonic cadence and presented PTS:
wait for a future epoch; return on-time inside hysteresis; use an absolute,
bounded proportional rate outside it; seek near a loop boundary above 80 ms;
hard-resync above 250 ms or after a gap above 2 seconds. Corrective seeks/hard
resyncs have a five-second cooldown, except initial start/interruption. A rate
command replaces the prior rate; it does not multiply it. Seeking adapters
must pre-roll and reset rate to 1.0. Flash-free application is UNVERIFIED.

`frame_for_target` selects the latest eligible **unwrapped** PTS from a bounded
sorted queue and lets the adapter discard older frames. A decoder running at
24 fps does not acquire a growing FIFO delay against a 60 fps group timeline.
Missing frames require repeat/poster/downshift policy, not slowing everyone.

Sleep/reboot/backwards monotonic changes and explicit authority takeover require
resetting the estimator/controller and incrementing generation, then a fresh
burst/pre-roll at current group PTS. Wall-clock time is not an input, so changing
UTC cannot affect playback. Losing packets alone free-runs the last map; reported
uncertainty grows with age. Disappearing coordinator means freeze the manifest,
not elect a replacement. The library exposes reset, not leader election.

Inputs are bounded (64 probes, 32 anchors, 256 frame entries; finite seconds
0..1e9, initial offset at most one day, exchanges at most two seconds).
Errors do not partially mutate state. Twelve tests cover signs, skew, low-RTT
selection, outliers, reorder/duplicates/replay/session fences, stale samples,
consistent lies, asymmetry, late join, future epochs, looping, sleep/backwards
clock, takeover, hysteresis, cooldown and slow decoding. Property-style seeded
tests exercise 100,000 arbitrary float/wire cases, valid-header bit mutations,
and 10,000 arbitrary estimator updates. This is bounded fuzz/property testing,
**not exhaustive formal verification or a continuous libFuzzer campaign**.

Mutation checks actually changed and restored five implementations: offset sign,
highest instead of lowest RTT, rate sign, missing hysteresis, and drift sign.
All five compiled and caused real test failures; the restored 12-test baseline
passed. See `transcripts/wallpaper/mutation-check.txt` and its executable script.

A peer lying consistently about its clock passes statistical filters; a slowly
changing liar can slew the estimate over time. Bounds protect numeric/resource
invariants, not truth. Authentication establishes authority, not accurate time.

## B. Deterministic one-hour simulation

`examples/simulate.rs` has a seeded xorshift generator and four models. Every
case includes the coordinator among its 2, 3 or 4 computers. Followers start
2.7 seconds apart (up to 8.1 seconds), at alternating ±50 or ±200 ppm, with
per-second random-walk changes up to ±1 ppm, bounded at 1.5× nominal drift.
Coordinator is the identity clock with zero drift; opposing nonzero coordinator
and follower drift is **UNVERIFIED by this harness**. Core tests independently
check positive and negative drift. Renderers sample 60 fps source PTS with
fixed independent 0/3/6/9 ms presentation phases and ideal timely decoding.
This is quantized displayed-source-PTS modeling, not a real compositor model.

| Model | Per-direction delay/loss assumptions |
|---|---|
| Wired | Uniform 0.1–0.3 ms each way (0.2 ms mean) |
| Good Wi-Fi | Uniform 1–5 ms each way (3 ms mean) |
| Bad Wi-Fi | Uniform 1–9 ms base; reverse adds 2–10 ms; independent 12% 30–150 ms spikes, plus staggered two-second reverse spike bursts every 45 seconds; 4% probe loss |
| Adversarial | Forward-only 120–150 ms multiplied by follower index+1; reverse zero; intentionally more asymmetric than the bad-Wi-Fi case |

All cases lose the network from simulated second 1800 through 1805 while
playback continues. This is **not suspend**; sleep behavior is tested separately.
Eight timestamp exchanges span 140 ms per burst; arrivals are scheduled, so
the estimator cannot consume future replies. Simulated server processing is
0.1 ms. Probe intervals 1/5/10/15 seconds × caps ±0.5/1/2% × drifts 50/200 ppm ×
2/3/4 computers × 8/32 anchors × seeds 42/1337 × four models = 1,152 cases.

Selected run: **four computers, 200 ppm, five-second probes, ±0.5% cap,
32 anchors, seed 42**. Error is the largest pairwise circular displayed-PTS
difference per global frame, not pooled per-peer clock-fit error. Statistics
include startup and outage; warm P95 after 60 seconds is identical here.

| Model | Mean ms | P50 ms | P95 ms | P99 ms | Max ms | ≤50 ms P95? |
|---|---:|---:|---:|---:|---:|---|
| Wired | 7.4204 | 0 | 16.6667 | 33.3333 | 33.3333 | Yes, model only |
| Good Wi-Fi | 7.0700 | 0 | 16.6667 | 33.3333 | 33.3333 | Yes, model only |
| Bad Wi-Fi | 8.1929 | 0 | 16.6667 | 16.6667 | 33.3333 | Yes, model only |
| Adversarial | 253.0248 | 250 | 266.6667 | 266.6667 | 266.6667 | **No** |

| Model | Rate-command changes | Boundary seeks | Hard resyncs incl. starts | Correction jumps >frame after first second | Displayed-source advances >frame | Initial convergence s | Outage recovery s | Max during outage ms |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Wired | 14211 | 0 | 4 | 0 | 233 | 0.150 | 0 | 16.6667 |
| Good Wi-Fi | 13992 | 0 | 4 | 0 | 198 | 0.150 | 0 | 33.3333 |
| Bad Wi-Fi | 16013 | 0 | 4 | 0 | 138 | 0.200 | 0 | 0 |
| Adversarial | 15804 | 0 | 4 | 0 | 83 | Never | Never | 250 |

Convergence means the beginning of the first one-second streak at ≤50 ms
with all mappings ready; it is not a forever-stable guarantee. Recovery is the
same streak measured from second 1805; zero means the outage did not push the
run out of tolerance. Four hard resyncs are initial joins, including coordinator
initialization. Rate counts are changed numerical rate commands, not entries
into correction mode. Display-frame advances include ordinary rate-induced
skips and initial joins; correction jumps subtract normal bounded playback
advance and exclude the first second. Both are reported to avoid hiding visible
motion discontinuities behind a zero-seek claim.

### Parameter sweep and choice

Across all 288 cases per model, worst P95 was 16.6667 ms for each of wired,
good Wi-Fi and bad Wi-Fi; worst P99/max were 33.3333 ms. None failed the P95
target. All 288 adversarial cases failed: P95 up to 266.6667 ms, no convergence.
Worst initial convergence was 0.150/0.150/0.200 seconds for wired/good/bad;
all their outage recoveries were zero, and no post-start correction jumps
occurred anywhere in the sweep. Bad-Wi-Fi worst outage maximum was 16.6667 ms.

| Swept parameter | Choices | Observed worst P95 wired/good/bad ms | Adversarial worst P95 ms |
|---|---|---:|---:|
| Probe interval | 1, 5, 10, 15 seconds | 16.6667 at every choice | 266.6667 at every choice |
| Nudge cap | ±0.5%, ±1%, ±2% | 16.6667 at every choice | 266.6667 at every choice |
| History length | 8, 32 accepted bursts | 16.6667 at both choices | 266.6667 at both choices |

Five-second probing costs roughly 1.6 requests/second/follower while active,
versus eight at one second and 0.533 at 15 seconds; responses double the
message count. The ±0.5% cap suffices here; higher caps bring no demonstrated
P95 benefit and can skip more frames. Use 32 anchors to damp noisy estimates,
but this sweep does not demonstrate a statistically superior optimum. A failed
initial burst should retry at one second before normal five-second pacing:
**PROPOSAL, hybrid retry policy UNVERIFIED in these runs**. Startup must wait for
readiness rather than rely on an unconditional two-second deadline.

Longer/complete correlated loss, changing asymmetry, arbitrary refresh rates,
decoder stalls, moving to another AP, and oscillator temperature transients
beyond this model remain UNVERIFIED. The slow-decoder unit test is not a
distributed performance simulation. Two deterministic seeds are reproducible,
not confidence intervals over real networks.

## C. Actual two-process QUIC path

`examples/transport.rs` reuses `one-mk-net::Node`, ephemeral identities and SAS
pairing patterns from `crates/net/tests/loopback.rs`. Parent and spawned child
exchange 64-byte messages on real QUIC bidirectional streams. A bounded raw-UDP
proxy injects per-packet delay, loss and jitter without root/tc/netem; it does
not parse QUIC. This is **two example processes, not two production daemons**.
No installed daemon or configuration was used. New control messages remain
experimental and do not change `PROTOCOL_VERSION`, `StreamKind` or the codec.

Both processes read Linux CLOCK_MONOTONIC; server adds exactly three seconds,
so true coordinator-minus-client offset is known. Twenty eight-exchange bursts
per model yield **20 clock-error observations**, not 160 independent estimates.
Good proxy delays are 1–5 ms; bad adds 6 ms reverse asymmetry, 80 ms spikes
every 13th packet and loss every 29th; adversarial adds roughly 120 ms forward
only. This deliberately differs from the hour simulation's network model.

| Proxy model | Mean absolute offset error ms | P50 ms | P95 ms | P99/max ms | RTT P50 ms | RTT P95 ms |
|---|---:|---:|---:|---:|---:|---:|
| Wired | 0.0248 | 0.0180 | 0.0520 | 0.1371 | 2.8194 | 3.8976 |
| Good | 0.3533 | 0.2213 | 1.0899 | 1.2981 | 8.9880 | 11.2061 |
| Bad | 2.9546 | 2.9005 | 3.8910 | 3.9300 | 15.5099 | 77.9282 |
| Adversarial | 60.6457 | 60.6705 | 61.1159 | 61.3248 | 124.9191 | 126.9126 |

Wired injection was only 0.2 ms each way but actual RTT was about 3 ms: timer
granularity/scheduling matter. QUIC retransmission and stream framing are in
the measured path. This validates offset signs and actual transport plumbing;
it does **not** measure cross-process presented PTS or establish the design's
20 ms warm-loopback playback gate. Small-sample P99 is just the maximum here.

### Exact additive experimental wire proposal

All numbers little-endian; exact length 64 bytes. This private version is not
an allocation of a production control tag. Production must negotiate support
and bind session handles to the complete authenticated scene manifest.

| Bytes, half-open | Field |
|---|---|
| 0..4 | ASCII `WCLK` |
| 4 | Experimental version 1 |
| 5 | Kind 1 request, 2 reply |
| 6..8 | Reserved zero |
| 8..16 | `generation: u64` |
| 16..24 | `sequence: u64` |
| 24..32 | `t1: f64`, monotonic seconds |
| 32..40 | `t2: f64` |
| 40..48 | `t3: f64` |
| 48..56 | `t4: f64`, zero on wire, receiver stamps arrival |
| 56..64 | `session: u64`, connection-local manifest handle |

Request t2/t3/t4 are zero; reply t3≥t2 and t4 remains zero. Reject wrong length,
trailing bytes, magic/version/kind/reserved bits, nonfinite/out-of-range seconds
and invalid ordering. Receiver matches session/generation/sequence/t1 to an
outstanding request before accepting t4. Resource bounds and anti-replay are
also enforced in the estimator. No URLs, paths or media payloads occur here.
Production control-channel prioritization versus KVM input, manifest handle
negotiation, peer revocation and daemon dispatch are **UNVERIFIED/not implemented**.

## D. One decode, N crops, measured

Host: Intel i7-11800H, 8 cores/16 threads, about 30 GiB RAM; Linux x86_64;
GStreamer 1.28.2 runtime. Development `.pc` files absent, so no gstreamer-rs
was compiled. Only installed CLI tools were used. Each trial generated a tiny
120-frame, 60 fps I420 SMPTE H.264 clip with `x264enc` ultrafast/two threads,
then decoded with `avdec_h264 max-threads=2`. One decoder feeds `tee`, bounded
queues and distinct horizontal `videocrop` branches; comparison runs instantiate
N independent decoders, each feeding its own distinct crop. Sinks are fakesinks
with `sync=false` to measure throughput, not a desktop compositor.

Three repeats per configuration; means below. CPU is `(user+system seconds)/2
media seconds ×100`: 100% means one core required at 60 fps, **not percentage
of the whole 16-thread CPU**. RSS is peak process RSS in MiB. FPS is 120/wall
seconds, i.e. simultaneous timeline throughput, not summed decoder frames.

| Source pixels | Outputs / decoders | CPU % at 60 fps | RSS MiB | Throughput fps |
|---|---:|---:|---:|---:|
| 1920×1080 | 1 / 1 | 18.50 | 115.07 | 391.96 |
| 1920×1080 | 2 / 1 | 31.67 | 115.03 | 348.59 |
| 1920×1080 | 2 / 2 | 52.17 | 115.05 | 243.76 |
| 1920×1080 | 3 / 1 | 21.67 | 115.09 | 400.30 |
| 1920×1080 | 3 / 3 | 73.83 | 130.33 | 253.70 |
| 3840×2160 | 1 / 1 | 38.83 | 149.79 | 216.32 |
| 3840×2160 | 2 / 1 | 65.67 | 162.00 | 192.52 |
| 3840×2160 | 2 / 2 | 139.67 | 248.78 | 117.65 |
| 3840×2160 | 3 / 1 | 81.83 | 161.76 | 163.45 |
| 3840×2160 | 3 / 3 | 262.17 | 336.42 | 95.55 |
| 7680×2160 | 1 / 1 | 102.17 | 244.52 | 88.23 |
| 7680×2160 | 2 / 1 | 143.17 | 269.01 | 86.45 |
| 7680×2160 | 2 / 2 | 263.50 | 438.65 | 66.63 |
| 7680×2160 | 3 / 1 | 144.17 | 268.79 | 100.75 |
| 7680×2160 | 3 / 3 | 544.33 | 609.11 | 47.81 |
| 11520×2160 | 1 / 1 | 93.17 | 340.11 | 96.79 |
| 11520×2160 | 2 / 1 | 209.50 | 376.17 | 46.09 |
| 11520×2160 | 2 / 2 | 396.00 | 629.49 | 47.17 |
| 11520×2160 | 3 / 1 | 255.83 | 376.26 | **38.81** |
| 11520×2160 | 3 / 3 | 674.50 | 882.80 | **40.12** |

One decode/three crops versus three decoders reduces 4K CPU from 262.17% to
81.83%, and RSS from 336.42 to 161.76 MiB in this test. Wide media remain costly.
The nonmonotonic one-output 7680/11520 result and counterintuitive small-input
throughput differences are retained, not smoothed away: short low-complexity
clips, process startup/cache state and an unpinned shared host are noisy.
No statistical confidence interval or sustained complex-media guarantee is
claimed. Fakesinks omit GPU upload/compositing/scanout; RSS is not VRAM.
Installed VA/VAAPI/NV/V4L2 hardware decoder factories were not found;
**hardware decode capability/performance is UNVERIFIED and was not tested**.

### Crop correctness and private display

Two Xvfb sizes were exercised: **960×320 with two 480×320 windows**, and
**1440×480 with three 480×480 windows**. A single H.264 decode is frozen and
split, cropped in I420, converted with dithering disabled and shown with
`ximagesink`. Root screenshots are captured by `ximagesrc display-name=:187`,
never the session display. A separately decoded reference is used only for
validation, not the presentation pipeline. Window creation order is
nondeterministic, so the script matches distinct reference crops before placing
windows. A preliminary incorrect-order experiment failed with channel error
255; its transcript is retained. Final full-image and seam-column comparisons
have **maximum channel error zero**, at x=480 and x=480/960 respectively.

Screenshots: `transcripts/wallpaper/private-xvfb-960-2.png` and
`transcripts/wallpaper/private-xvfb-1440-3.png`. This proves spatial crop assembly
on Xvfb, not atomic refresh across windows or GNOME Wayland wallpaper placement.
An earlier alternate pixel-format experiment was inconclusive; its cause is
UNVERIFIED, not evidence of an upstream GStreamer defect. The final explicit
I420-crop-convert route is the measured one.

## E. Decoder licences, vulnerability surface and isolation

The exact local `gst-inspect-1.0` fields are retained in `plugins.txt` and
`extra-plugins.txt`; selected factories below all report version 1.28.2.
These are **plugin-reported licences**, not a licence audit of every linked
library, patent right or distribution arrangement.

| Factory | Plugin | Reported licence | Disposition |
|---|---|---|---|
| avdec_h264, avdec_h265, avdec_gif, avdec_apng | libav | LGPL | Broad FFmpeg surface; linked build is GPL-enabled; not an automatic shipping choice |
| openh264dec | openh264 | BSD | Candidate H.264-only path after profile/security/licensing validation |
| vp8dec, vp9dec | vpx | LGPL | Candidate explicitly selected VP8/VP9 |
| av1dec | aom | LGPL | Optional AV1; throughput and resource policy UNVERIFIED |
| libde265dec | de265 | LGPL | Optional HEVC, not initial allow-list |
| pngdec | png | LGPL | Candidate bounded static PNG after patch verification |
| webpdec | webp | LGPL | Candidate bounded WebP after patch verification |
| gdkpixbufdec | gdkpixbuf | LGPL | Reject broad image-loader autoplugging initially |
| h264parse | videoparsersbad | LGPL | Parser attack surface still needs a patched build |
| qtdemux / matroskademux | isomp4 / matroska | LGPL | Explicit container gates, not trusted input |
| x264enc | x264 | GPL | Used only for local synthetic benchmark generation |

`dav1ddec` is absent even though the system has libdav1d. The local decoder
inventory is much broader than this proposed allow-list. `playbin3` availability
does not imply it is safe to let it choose every installed decoder.

**Important measured licence discrepancy:** `libgstlibav.so` links system
libavcodec62 (package `7:8.0.1-3ubuntu2`). Calling that library's
`avcodec_license()` returns `GPL version 2 or later`; configuration contains
`--enable-gpl`. The wrapper's LGPL field therefore does not establish an
LGPL-only delivered dependency graph. Do not bundle/link this build assuming
it preserves the product's MIT/Apache-only obligations. Exact redistribution
and subprocess combined-work conclusions require review: **UNVERIFIED**, not
legal advice or a claim that every installed codec has the same licence.

### Concrete CVE surface, not a clean bill of health

The GStreamer upstream security index and advisories were read on 2026-10-30.
Examples relevant to the proposed media path:

- [SA-2026-0022](https://gstreamer.freedesktop.org/security/sa-2026-0022.html):
  CVE-2026-39043, Matroska heap overflow, upstream fixed in
  1.28.2. Installed upstream base includes that version; binary verification
  and a complete distro backport audit remain UNVERIFIED.
- [SA-2026-0047](https://gstreamer.freedesktop.org/security/sa-2026-0047.html):
  CVE-2026-12892, H.264 extension-header NAL one-byte out-of-bounds read, upstream fixed
  in 1.28.5. Installed base 1.28.2 is older; exact local backport status
  **UNVERIFIED**. This is in the parser path even if the decoder changes.
- [SA-2026-0052](https://gstreamer.freedesktop.org/security/sa-2026-0052.html):
  CVE-2026-18298, gdkpixbufdec heap overflow after image dimension changes,
  upstream fixed in 1.28.5. Installed good-package changelog explicitly lists
  a backport and rank reduction; binary-level verification **UNVERIFIED**.
- [SA-2026-0079](https://gstreamer.freedesktop.org/security/sa-2026-0079.html):
  CVE-2026-88914, qtdemux caption-box integer overflow/out-of-bounds read,
  upstream fixed in 1.28.7. Installed good-package changelog explicitly lists
  the same CVE backport; binary-level verification **UNVERIFIED**.

Local package changelogs prove that version-string comparison alone is
insufficient: bad `1.28.2-1ubuntu1.2` lists CVE-2026-19387 ADPCM fixes and
CVE-2026-53701/53702 H.266/H.265 fixes; good `1.28.2-2ubuntu0.4` lists PNG,
MP4, FLAC, metadata, RTP and WavPack security changes. These packaging records
are not proof that every above advisory is fixed, nor an exploit test. The
retained excerpts are bounded, not a complete change history. Current FFmpeg,
OpenH264, libvpx, libpng, libwebp, AOM and transitive-library vulnerability
coverage is **UNVERIFIED**. No claim of an exhaustive CVE list or safe installed
set is made. Upstream FFmpeg security page retrieval failed during research.

Product recommendation: allow **specific factory + container + codec/profile +
resolution/frame-rate limits**, not plugin families or URI autoplay. Start with
audited/patched H.264 through OpenH264 or a deliberately audited LGPL-only build,
and VP8/VP9; PNG/WebP only with bounded dimensions/animation budgets. GIF/APNG
need a separately audited bounded decoder before enabling, not automatic libav
fallback. Keep HEVC/AV1 optional until resource/patent/platform tests pass.
Candidate means **not yet approved for release**.

Run parsing and decoding in an unprivileged, bounded helper with no network,
pairing keys, broad filesystem access or compositor privileges, using an
already-open authorized file and bounded frame IPC. Enforce CPU/RSS/queue
limits and crash backoff; never load media libraries into KVM daemon or Shell.
An allow-list is not a sandbox; hardware decode is not a sandbox either.
Actual seccomp/namespace/Windows/macOS isolation remains **UNVERIFIED and
unimplemented in this spike**. Retain software/static fallback and explicit
failure reasons. Licences and current CVEs must be reviewed per shipped binary.

## Changes required in the design

1. Promote the I/O-free estimator/controller/wire validation to a reusable
   prototype, but leave 50 ms P95 as a **qualified physical-lab release gate**.
   Publish asymmetry uncertainty and degraded status independently of RTT.
2. Specify 8×20 ms active-only bursts, five-second refresh, readiness before
   epoch, generation/session fences, 8/3 ms hysteresis, ±0.5% default and
   five-second seek cooldown. Faster initial retry is a proposal, not measured.
3. Distinguish displayed-source frame skips, correction seeks and actual
   physical seam artifacts. The rate API is absolute; adapters must reset after
   seeks and invalidate on suspend. Never let a slow decoder slow the group.
4. Preserve freeze-manifest/free-run disconnect policy and explicit takeover.
   Add real KVM-load/control-priority and authority-revocation tests before
   putting the experimental envelope on a production control stream.
5. Keep one decode per device, with bounded queues and I420 crop then conversion
   as a verified software route. Correct three-4K dimensions to 11520×2160;
   retain default admission around 4096×2160 and require measured opt-in for wide
   assets. No software 60 fps promise for true three-4K on this host.
6. Treat the installed libav GPL build and unresolved patch status as packaging
   gates. A fresh allow-listed decoder helper, not global autoplugging, is needed.
7. Section 7's disabled/no-helper/no-heartbeat power invariant remains required;
   these explicit examples are not production idle/power measurements. No
   native bridge, hardware decode or sandbox release gate is waived.

Recommended next step: an explicitly authorized isolated-session/hardware lab
with two real Wi-Fi computers, high-speed seam capture and presented-PTS logs,
KVM traffic under load, sleep/resume/takeover and complex-media decoder stalls.
First qualify the sandbox and shipped decoder build. Do not install a bridge
or change the owner's live wallpaper to perform that follow-up implicitly.

## Reproduction and retained transcripts

Run from repo root, with installed tools only. Never run `cargo fmt` or a release
build. Loopback tests/proxy and private Xvfb may need permission outside the
filesystem sandbox. No root networking configuration is needed.

```sh
git show t3/research-live-wallpaper-design:docs/WALLPAPER-DESIGN.md
cargo test -j 2 -p one-mk-wall-sync
cargo rustc -j 2 -p one-mk-wall-sync --example simulate -- -C opt-level=2
target/debug/examples/simulate > docs/spikes/transcripts/wallpaper/simulation.csv
cargo run -j 2 -p one-mk-wall-sync --example transport > docs/spikes/transcripts/wallpaper/transport.csv
python3 docs/spikes/transcripts/wallpaper/measure-gst.py
python3 docs/spikes/transcripts/wallpaper/summarize.py
python3 docs/spikes/transcripts/wallpaper/mutation-check.py
cargo test -j 2 --workspace
cargo clippy -j 2 --workspace --all-targets -- -D warnings
cargo clippy -j 2 --target x86_64-pc-windows-msvc -p one-mk-wall-sync --all-targets -- -D warnings
cargo clippy -j 2 --target aarch64-apple-darwin -p one-mk-wall-sync --all-targets -- -D warnings
git diff --check
```

Mutation checks must run alone, not concurrently with builds or measurements.
Cross-target clippy compiles/checks all crate targets but **does not link or run
Windows/macOS binaries**. The transport example is Linux-only with explicit
foreign-target stubs. This is not native-platform runtime validation.

The GStreamer script supports `--benchmark-only` and `--screenshots-only`, refuses
an occupied :187, cleans up Xvfb/pipelines in finally blocks, and uses installed
Pillow/X11 for image comparison/window placement. Its exact generated CLI
arguments are in the script; representative two-crop pipeline is:

```sh
gst-launch-1.0 -q filesrc location=target/wall-spike/input.h264 ! h264parse ! avdec_h264 max-threads=2 ! tee name=split \
  split. ! queue max-size-buffers=2 ! videocrop left=0 right=1920 ! fakesink sync=false \
  split. ! queue max-size-buffers=2 ! videocrop left=1920 right=0 ! fakesink sync=false
```

The representative input path is generated by the script under width-specific
names and deleted afterward; use the script for a complete reproduction.
No downloaded media is required. Decoder inventory/licence checks:

```sh
gst-inspect-1.0 --version
for factory in playbin3 appsink glimagesink avdec_h264 avdec_h265 avdec_gif avdec_apng openh264dec vp8dec vp9dec av1dec libde265dec pngdec webpdec gdkpixbufdec h264parse qtdemux matroskademux x264enc; do
  gst-inspect-1.0 "$factory"
done
ldd /usr/lib/x86_64-linux-gnu/gstreamer-1.0/libgstlibav.so
python3 - <<'PY'
import ctypes
library = ctypes.CDLL('libavcodec.so.62')
for symbol in ('avcodec_license', 'avcodec_configuration'):
    function = getattr(library, symbol)
    function.restype = ctypes.c_char_p
    print(function().decode())
PY
```

Evidence directory `docs/spikes/transcripts/wallpaper/` contains the complete
simulation/transport/60-row GStreamer CSVs, derived `summary.txt`, source scripts,
crop comparison text and PNGs, plugin/linked-library/changelog records, mutation
test failures and restored success, workspace tests and all three clippy logs.
Empty GStreamer stdout/stderr logs indicate no emitted errors, not missing
measurements. Sandbox and preliminary crop-order failures are retained rather
than concealed. Final requested tests/clippy pass; no formatting was run.

Primary external references: [GStreamer security index](https://gstreamer.freedesktop.org/security/)
and [licensing FAQ](https://gstreamer.freedesktop.org/documentation/frequently-asked-questions/licensing.html).
Installed binaries and local package records, not upstream labels alone, drive
the packaging warning. All remaining deployment claims are explicitly UNVERIFIED.
