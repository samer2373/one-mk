# Wallpaper canvas and receiver consent: M1c scaffolding

Implementation scope, 2026-10-30. Read alongside
[design rev 2](WALLPAPER-DESIGN.md), especially sections 3, 4, 7–10 and 12.
**This is not playable wallpaper and does not complete M1.** The
[native presenter stop](spikes/wallpaper-m1-stop.md) remains in force. This work
does not run or modify a desktop compositor, launch the probe, import files,
load a decoder, or implement a scene prepare/commit protocol.

## Pure geometry model

`one-mk-wall-canvas` is a dependency-free, I/O-free reference implementation,
separate from the existing `one-mk-wall-sync` estimator/controller/wire crate.
The daemon does not yet connect these two cores to a playback pipeline.

`Layout::new` validates already-global logical output rectangles.
`Layout::resolve` adds explicit relative computer origins to computer-local
output positions before the same validation. Each computer must resolve to an
anchored origin. Missing offsets, side-only adjacency and unanchored consistent
cycles are ambiguous; nonzero-sum cycles are inconsistent. Neither is guessed.

Limits: 8 computers, 16 outputs, nonempty device/screen IDs at most 256 UTF-8
bytes without control characters, and every rectangle edge within
[-1,000,000, +1,000,000] logical units. Dimensions are positive and arithmetic
is checked before global placement. Duplicate identities and every positive-area
overlap, including mirrored outputs, are errors. Touching edges are allowed.
These deliberately reject mirrored layouts, rather than promising duplicate
presentation. There is no file or display discovery in this crate.

Coordinates can be negative. Gaps and bezel space are real scene space.
The bounds are the bounding rectangle of all reserved outputs, not a packed
sum of their widths. **A missing/asleep computer's rectangle stays reserved.**
Callers must retain those outputs in the committed layout; there is no online
filter or auto-reflow. Removing/repositioning them requires an owner decision.

Each output records its oriented logical rectangle, scale (1, 5/4, 3/2, or 2),
rotation (0/90/180/270 clockwise) and unrotated physical buffer dimensions.
Physical buffer dimensions must exactly match logical size times scale, with
axes swapped for portrait orientation. Fractional extents requiring an OS-specific
rounding guess are refused. Physical-DPI calibration is not implemented.

### Exact sampling and integer rounding

All transforms use reduced signed i128 rational numbers; no floating-point
rounding occurs in geometry. The bounds' minimum x/y is the shared scene origin.
A buffer pixel at integer (x,y) samples its centre, ((x+1/2)/scale,
(y+1/2)/scale), then output rotation and the global output position are applied.
Rectangle ownership is half-open, [left,right) × [top,bottom).

Source sample-aspect ratio expands source width before source rotation and
scene fitting. Source dimensions are 1..1,000,000 and each SAR term is
1..10,000. The four fits are global, not independently fitted per output:

| Fit | Rule |
|---|---|
| Cover | Larger of canvas/display width and height ratios; centre crop |
| Contain | Smaller ratio; centred image, opaque padding outside |
| Stretch | Independent width/height ratios, no aspect preservation |
| Tile | One source-display unit per logical unit, anchored at canvas minimum; Euclidean positive modulo on both axes |

`sample` returns exact raw-source pixel-edge coordinates or padding.
`crop` provides nearest-neighbour RGBA reference pixels: floor each coordinate
after the inverse SAR/rotation transform. If an inverse rotation maps an exact
tile boundary to the far source edge, choose the final texel (min(floor, size−1)),
not the next row or an out-of-bounds index. Fit inclusion is checked first, so
this rule does not turn out-of-image contain padding into edge stretching.
Padding alpha must be 255. Input and each output reference buffer are bounded
to 16,777,216 pixels; larger validated logical layouts do not allocate an image.
This is a CPU spatial oracle, not a GPU/chroma/filtering/color-space contract.

Goldens are explicit tiny RGBA arrays in `tests/geometry.rs`. Generated property
cases compare adjacent monitor sample coordinates with a single full-canvas
sample transform for every
fit over widths 1..16 and heights 1..12. Further tests cover union ownership
including intentional holes, determinism, density/rotation, negative origins,
SAR extremes, bounds and relative placement errors. No claim is made that
intentional bezel holes are filled by monitors. Removing origin normalization,
using contain's minimum for cover, and removing pixel-centre offsets were each
deliberately mutated: all three mutations were killed by the tests, then restored.
A failing rotated-tile edge regression exposed and fixed a real indexing bug.

## Additive bounded wire envelope

Existing protocol version remains 2. `caps::WALLPAPER` is bit 256, and control tag
25 carries one wallpaper message. Existing old link readers ignore unknown
control tags. Updated daemons only announce wallpaper after the peer advertises
the capability **and** the local receiver has enabled the feature and explicitly
granted that peer consent. A legacy-capability real peer is tested to receive
no wallpaper announcement. This is not an old-release binary interoperability lab.

The control payload is exactly 113 bytes: tag plus this 112-byte envelope.
Existing control framing and its 16 KiB ceiling are unchanged. A malformed
wallpaper envelope closes that control link without per-frame warning spam.

| Envelope bytes | Encoding |
|---|---|
| 0..2 | Version 1, u16 little-endian |
| 2 | Kind: 0 version, 1 renderer, 2 canvas, 3 consent request, 4 grant, 5 revoke |
| 3..8 | Reserved; must be zero |
| 8..40 | Full coordinator fingerprint, 32 bytes |
| 40..56 | Nonzero receiver-issued random 128-bit session epoch |
| 56..64 | Nonzero strictly increasing u64 little-endian sequence |
| 64..112 | Kind-specific 48-byte descriptor |

Canvas descriptor: width/height u32 LE at offsets 0/4, each 1..2,000,000;
computer/output counts at 8/9, 1..8 and computer-count..16 respectively;
6 zero reserved bytes; 32-byte opaque descriptor hash at offset 16.
Renderer descriptor: availability byte 0 or 1 and 47 zero bytes. This build
announces unavailable (0). Other descriptors must be all zero. Exact lengths,
version, kinds, reserved fields and bounds are checked before admission.

This canvas descriptor is **not** the design's full bounded/canonical canvas
manifest. Its hash is not authenticated scene approval or independently verified
geometry. The only stored peer metadata is one 48-byte pending descriptor per
consented peer. No URL, path, command, content/file bytes, or media transfer kind
exists here. Full manifests, provenance, compatibility negotiation and the
prepare/commit state machine remain future work.

## Receiver-owned consent and CLI

Run on the computer that would display the scene:

```sh
one-mk wallpaper status --json
one-mk wallpaper enable
one-mk wallpaper consent grant PEER
one-mk wallpaper consent revoke PEER
one-mk wallpaper disable
one-mk wallpaper doctor --json
```

PEER is one directly paired computer's name, unambiguous fingerprint prefix or
full fingerprint as supported by the trust store. Consent is not transitive.
Revoke also accepts a full saved canonical fingerprint. Persistent policy lives
on the receiving computer, updated through the existing atomic settings lock/save:

```toml
[wallpaper]
version = 1
enabled = false

[wallpaper.grants]
# Full lower-case 64-character fingerprint = nonzero receiver consent revision
```

Existing settings upgrade to disabled/no grants without changing unrelated
settings. Unknown policy versions/fields, malformed fingerprints, zero revisions
and more than eight grants fail closed. No peer message writes this file or
grants permission. A locally paired computer does not automatically have consent.
Forgetting a computer also removes its persisted wallpaper grant; an unreadable
policy file must be repaired before forgetting completes, rather than retaining
a stale grant for a future re-pair.

Each grant/regrant creates a fresh consent revision. Session challenges rotate
on regrant, restart, re-enable and connection replacement/disconnection. Admission
requires the current authenticated direct link, negotiated capability, local
enabled policy, explicit receiver consent, matching TLS peer/coordinator
fingerprint, matching epoch and strictly increasing sequence. Incoming grant
messages cannot self-authorize. Version/renderer/grant announcements currently
advertise the receiver challenge only; they are not a complete bidirectional
scene protocol. A received revoke stops the current session and clears its
pending descriptor; only a local regrant revives that session. This remote stop
does not silently rewrite the owner's persisted grant.

Local revoke and disable clear pending metadata. Disable preserves saved grants
for a later explicit enable; revoke removes the selected grant. Neither operation
depends on a peer acknowledging it. A policy-save/daemon-apply failure returns
an error telling the operator to stop the daemon before continuing.

Exit codes: 0 success; 2 validation/usage; 3 unavailable renderer; 4 absent or
ambiguous direct authority; 5 operational failure. `--json` emits a success/error
object on stdout; failures also produce the normal stderr diagnostic. Commands
use the existing finite local IPC timeout. Status explicitly says scaffolding,
unavailable renderer, no playback; normal status and doctor include a wallpaper
line. Enabled wallpaper doctor returns 3, off returns 0. A successful enable
only enables policy, never promises playback.

## Threat model and idle behavior

The threat is an authenticated but hostile paired peer, including spoofed
identities, replay, stale epochs, descriptor flooding, oversized messages and
permission withdrawn between messages. Wallpaper authorization does not replace
the existing KVM file/clipboard policy: those are separate purposes. Since no
wallpaper content/command/file route exists, none can be accepted through this
scaffold with or without consent. Future media routes must use the same local
gate before allocation, file creation and every commit, not reuse metadata
admission as playback authority.

Ingress admits at most 64 wallpaper messages per second per link before enqueue
to the existing daemon channel. The receiver admits at most 16 per second per
consented peer (invalid attempts consume the budget too). Both windows are
message-driven, with no scheduled timer. There are at most eight receiver states;
accepted/rejected totals saturate. This limits wallpaper ingress per link, not
the existing global connection count or all KVM message traffic. A continuously
active hostile peer still costs network/decoder CPU; zero idle means quiescent
operation, not zero work under attack. Local trust/settings tampering and a
compromised owner account are outside this authorization boundary.

Off has no wallpaper epoch state, active time origin, timer, child, content or
announcement. The capability bit is still additive in ordinary KVM capability
exchange. Enabled scaffolding is also passive, with no renderer or scheduled
work. An isolated real null daemon is sampled off/on/off for two seconds each
after settling: each must consume **0 process CPU ticks**, and report zero
wallpaper timers/children. This short no-peer sample is not a long power test,
does not cover active network traffic, and is not a hardware battery result.

## Validation and remaining gates

Tests were developed failing-first for geometry, wire and receiver state. The
real-binary Linux test suite starts three isolated null-backend daemons plus a
raw authenticated QUIC test peer, never an owner's daemon. Private homes and
runtime directories, `env_clear`, no discovery/clipboard/notifications, and
PID-owned teardown keep it away from the live desktop. It exercises default
off/no consent, nontransitive grants, admitted descriptor metadata, spoof/replay/
wrong epoch, flood rejection, local revoke between messages, regrant,
SIGKILL/restart with persisted policy/fresh challenge, oversized envelope link
closure, old settings preservation, legacy capability silence and forget cleanup.

Reproduce with a clean environment, private HOME and normal Rust toolchain cache;
loopback test permission is required. Never use `cargo fmt` for this task.

```sh
cargo test -j 2 --workspace --offline
cargo test -j 2 -p one-mk --test wallpaper_e2e --offline -- --nocapture
cargo clippy -j 2 --workspace --all-targets --offline -- -D warnings
cargo clippy -j 2 --target x86_64-pc-windows-msvc --all-targets --offline \
  -p one-mk-input -p one-mk-proto -p one-mk-ctl -p one-mk-app -p one-mk-tray \
  -p one-mk-wall-canvas -p one-mk-wall-sync -p one-mk-wall -- -D warnings
# Repeat the cross-clippy command for aarch64-apple-darwin.
```

Cross-clippy is compile/lint evidence only, not Windows/macOS daemon execution.
Not tested or implemented: media mid-transfer revocation (only pending metadata
withdrawal exists), hostile media/import/sandboxing, transfer resume and atomic
rename/commit, live Layout-to-manifest integration, GPU crops/chroma/color,
cross-machine physical seams/sync, native OS presentation, lock/sleep/hotplug,
ten-minute power and 24-hour soak, or independent security review. No Mutter/GNOME
harness was run in this task. These omissions keep the full M1 exit criteria
**NOT MET**, despite the tested geometry and receiving-policy groundwork.

### Recorded results (2026-10-30)

| Check | Result |
|---|---|
| Clean-environment workspace tests | PASS; existing ignored tests remain ignored |
| Workspace all-targets clippy, warnings denied | PASS |
| Windows MSVC cross-clippy, package list above | PASS; compile/lint only |
| macOS ARM64 cross-clippy, same package list | PASS; compile/lint only |
| Geometry tests | 10 passed, including generated seam/ownership properties |
| Real-binary wallpaper integration tests | 3 passed |
| Isolated daemon CPU, off/on/off | 0 / 0 / 0 process ticks, each over 2 seconds |
| Deliberate geometry mutations | 3 killed by behavioral test failures, restored |
| Failing-first evidence | Missing geometry/wire/policy APIs fail compilation before implementation; rotated tile edge then fails at runtime before its fix |

Raw local command logs are retained in ignored `.lab/` in the implementation
checkout (`workspace-tests-final.log`, `clippy-linux-final.log`,
`clippy-windows.log`, `clippy-macos.log`, `e2e-final.log`, the `*-red.log` and
`mutation-*.log` files). They are not release artifacts or independent review.
