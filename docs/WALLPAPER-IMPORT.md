# Wallpaper import: M1b development implementation

Evidence date: **2026-10-30**. Baseline: `wave5`, commit `76534af`.
This is an optional, finite **source admission and poster** pipeline, not a
wallpaper presenter, normalized cache, consent implementation or M1 sign-off.
The existing presentation stop is unchanged; no Mutter/GNOME harness was run.

## Trust boundary and process lifetime

The daemon does not depend on `one-mk-wall-import`, `image`, or GStreamer for
this feature. Its explicit `wallpaper` command dispatches before configuration,
identity, logging or the async runtime; it executes only a fixed sibling helper
with a cleared environment. Missing helper/runtime reports unavailable, not a
background retry. General `doctor` is passive; explicit wallpaper `doctor`
actually creates a short-lived sandbox and verifies denial. No off-state timer,
watcher, downloader, display connection, clipboard access or service is added.

The parent treats source bytes as opaque: opens a regular file using an
`openat(O_NOFOLLOW)` component walk, refuses `..` and symlinks, copies to a
private staging directory with a streaming BLAKE3 hash, then passes only that
snapshot descriptor into an isolated worker. The output directory must already
exist, belong to the caller and have mode 0700. A nonblocking directory flock
serializes imports targeting that directory. It is not a machine-wide queue.

Linux requires `/usr/bin/bwrap`, working user/PID/network/IPC/UTS namespaces and
seccomp. The worker sees only a read-only **complete runtime root**, its own
executable and one read-only input, private proc/dev and a 16 MiB scratch tmpfs.
No host home, document tree, display socket, render node or network capability
is mounted. The input worker installs `no_new_privs` and architecture-checked
seccomp before inspecting bytes. Sockets/connect/bind/listen, namespace/mount
changes, ptrace, process-VM operations, io_uring, keyctl, bpf and alternate x32
syscall entry are denied. Only anonymous AF_UNIX socketpairs remain for
GStreamer internal wakeups; no external endpoint can be opened.

Image decoding cannot create subprocesses. WebM decoding creates one fixed
`/worker --decode-webm` wrapper, which installs restrictions before executing
the pinned gst-launch pipeline; subsequent process creation is denied (thread
clones remain allowed). There is no decodebin, URI loader or encoder in that
pipeline. GStreamer plugin registry/search paths are private and explicit;
registry scanning does not fork. PID namespace teardown and bubblewrap's
parent-death contract kill descendants on worker/launcher death. The parent
drains bounded output/error pipes, kills on timeout/cancel/overflow, and waits
for both child and reader threads. SIGINT/SIGTERM cancellation returns 130.

**Fallback is refusal.** No ordinary-process, host-GStreamer, FFmpeg, privileged
daemon or unsandboxed import fallback exists. Namespace/seccomp/runtime setup
failure produces `sandbox unavailable: refusing to import`. A Linux platform
without this bubblewrap configuration is unavailable, even if another sandbox
technology might be installed.

## Admission, output and limits

Accepted: VP9 or AV1 in WebM, animated GIF, APNG and animated WebP. Static images,
MP4/H.264/H.265, AVI, Ogg, general Matroska and unsupported codecs get a clear
`convert first` error. Malformed supported files get validation errors instead
of being retried through another decoder. The WebM subset intentionally rejects
lacing, BlockGroup, encrypted/encoded tracks, unknown track-reference fields,
extra video/audio/subtitle tracks and attachments. A finite Info Duration must
agree with the inferred frame timeline within 2 ms. Some otherwise-valid WebM
files will therefore require conversion. Container timestamps are validated;
decoder PTS normalization is **not implemented**.

| Bound | Enforced policy, not a measured throughput guarantee |
|---|---|
| Source and total committed output | each at most 512 MiB; growing source is stopped |
| Canvas | positive dimensions, at most 4096 × 2160 / 8,847,360 pixels |
| Timeline | 2–36,000 frames, at most 600,000 ms, source at most 60 fps |
| Image frame delay | at least 17 ms; faster/zero-delay animations require conversion |
| Total decoded work | at most 1,000,000,000 pixels; deliberately stricter than duration alone |
| Image decoder allocation | 256 MiB image-library allocation limit; frames streamed/disposed |
| Uncompressed poster | opaque black alpha flattening, at most 320 × 180 RGB PPM |
| Metadata | at most 256 KiB before passing to decoders |
| Per-process virtual memory | RLIMIT_AS 1 GiB, inherited by decoder |
| Per-process CPU / wall time | 120 CPU seconds / 600 wall seconds (doctor: 5 seconds) |
| File / descriptors / cores | 512 MiB per file, 64 descriptors, no core dumps |
| Worker result / stderr | 1 MiB / 16 KiB; overflow tears down worker |

The source and poster are rehashed in the parent. Strict bounded worker JSON
must have matching valid dimensions/frame/duration fields. Source, poster and
record use create-new files (0600) in a 0700 directory; fsync precedes Linux
`renameat2(RENAME_NOREPLACE)`, then output-directory fsync. Content directories
are named by source BLAKE3. Existing objects, including symlinks, are never
overwritten. `record.json` contains schema, source/poster hashes, source size,
format/dimensions/frame count/duration and a fixed qualification warning. It
contains no timestamps, absolute source path or owner metadata; repeating an
import into an empty output produces the same record and poster hash.

Failure before commit removes staging through RAII. A parent SIGKILL can leave
an unpublished `.import-*` directory; owned private partials older than 24 hours
are cleaned at the next explicit import, never by an off-state timer (bounded
to the first 4,096 directory entries per activation). Completed objects survive
failure. A directory-sync error after rename explicitly reports that commit
already happened; never blindly retry as though nothing persisted.

## CLI contract

```text
one-mk wallpaper import FILE --runtime AUDITED_ROOT --output PRIVATE_DIR [--json]
one-mk wallpaper doctor [--runtime AUDITED_ROOT] [--json]
```

Runtime paths must be absolute, canonical and nonsymlink. No URL imports or
implicit output/config directory exists. Codes: 0 completed/available; 2
usage/validation; 3 not-ready/sandbox unavailable; 4 refused path authority; 5
operational failure (including killed worker); 130 explicit cancellation.
Success JSON is the completed record, never an accepted-but-unfinished promise.
Error JSON contains `error` and `exit_code`; doctor JSON has `available/reason`.

## Runtime and licence/distribution gates

GStreamer is pinned to **1.28.2**. `runtime.json` schema 1 inventories every file
with SHA-256 and a licence expression. Unlisted, changed, symlink/device or
group/world-writable runtime members are refused. Runtime identity is an
**administrator/packager trust input**, not something obtained from the media.
The manifest is not signed: it does not authenticate a malicious packager or
prevent same-user concurrent mutation. A production immutable signed package
and provenance audit remain required.

`scripts/check-wallpaper-artifact.py` traverses the importer Cargo closure,
checks SPDX AND/OR/WITH expressions, and optionally inventories the **entire**
runtime, checks ELF NEEDED closure/RPATH and requires exactly these plugins:
coreelements, matroska, vpx, aom, videoconvertscale. FFmpeg/libav/libswscale,
libswresample, mpv, x264/x265 and unknown/nonapproved licences fail closed.
The explicit plugin allowlist also rejects other encoder plugins. vpx/aom
plugins contain permissive royalty-free encoder factories, but the worker
pipeline invokes only decoders. Names alone are not a source/binary provenance
audit; a renamed or falsely licensed malicious binary defeats a manifest-only
classification. No GPL encoder or libav is authorized for distribution.

LGPL dynamic runtime components are distinct from permissive Rust dependencies.
The GCC runtime exception is allowed only as the explicit SPDX exception, not
as a blanket GPL permission. Shipping still requires exact licence notices,
source/build offers and the LGPL replacement test specified in the design.
MIT/Apache licensing of this application does not erase those obligations.

The gate's self-test injects an FFmpeg Cargo node **and an on-disk FFmpeg runtime
member with valid inventory hashes** and proves both fail; it also tests GPL,
unknown/malformed SPDX expressions and forbidden plugin names. `--helper`
executes the trusted build's `--build-info` and rejects fault-injection builds.
Use this explicit invocation on a candidate runtime before any future packaging:

```sh
cargo metadata --locked --format-version 1 > target/wallpaper-metadata.json
python3 scripts/check-wallpaper-artifact.py --metadata target/wallpaper-metadata.json \
  --helper target/debug/one-mk-wall-import --runtime /absolute/candidate-runtime \
  --sbom target/wallpaper-import.cdx.json
```

CI runs parser/property tests, clippy, negative gate tests and cargo-deny on the
new crate. The release workflow builds the optional helper and creates a
CycloneDX 1.5 **Rust/helper-only dry-run** SBOM as a workflow artifact. It neither
publishes the helper/runtime nor claims a complete shipping SBOM without
`--runtime`. Existing release binaries remain unchanged. The crate's
`deny.toml` includes advisories, compatible Rust licences, bans and source gates.

## Reproduction and evidence

Use a cleared environment with explicit Cargo/rustup homes; `cargo ... -j 2`;
never run cargo fmt. All generated media/runtime/tools belong under ignored
`target/wall-import-lab`, not the small host `/tmp` filesystem.

`scripts/wallpaper-lab-runtime.py ROOT --worker BINARY` creates a **lab-only**
snapshot from a matching trusted installed GStreamer. It is not a redistributable
runtime, supply-chain pin, licence audit, source offer or update mechanism.
Run `--help` for its exact arguments. Generate image fixtures using the crate's
`fixtures` example. The explicit sandbox test also expects two original
64 × 64, six-frame, 30 fps clips at `media/six.webm` (VP9) and
`media/six-av1.webm` (AV1), generated with videotestsrc and the matching
permissive encoder/webmmux. Never accept a downloaded test runtime implicitly.

```sh
ONE_MK_WALL_TEST_RUNTIME=/absolute/worktree/target/wall-import-lab/runtime \
  cargo test --offline -j 2 -p one-mk-wall-import --features test-faults -- --nocapture
cargo clippy --offline -j 2 --workspace --all-targets -- -D warnings
python3 scripts/check-wallpaper-artifact.py --self-test
```

Default CI explicitly logs that real sandbox tests are **not qualified** without
the runtime; an early return is not sandbox evidence. `test-faults` is empty by
default and must never ship. The real-binary test SIGSTOPs and then SIGKILLs the
worker at start/probe/decode/decoder-child/poster/response stages, and cancels
via SIGTERM. It verifies empty output and disappearance of recorded descendants.
Generated fixtures exercise truncation, huge dimensions, metadata bombs,
polyglots, wrong codecs/containers, output symlinks and source path tricks.
Seeded mutation tests and 256 proptest cases exercise raw/magic-prefixed inputs.
Two behavioral failing-first regressions found and fixed an EBML eight-byte
size overflow and missing predecode codec-metadata bounds.

Observed original WebM fixtures: VP9 **622 bytes**, AV1 **724 bytes**; both import
as six frames / 200 ms at 64 × 64. GIF/APNG/WebP each import as two frames /
200 ms at 2 × 2. Real Linux tests exercise available and unavailable sandboxes,
duplicates, deterministic records, cancellation and killed-worker cleanup.
No performance, 4K maximum-load, full ten-minute or 24-hour qualification is
claimed from these tiny fixtures.

### Validation record (2026-10-30)

| Check | Observed result |
| --- | --- |
| Default importer tests, cleared environment | 8 admission + 3 binary tests pass; runtime test explicitly unqualified without opt-in |
| Explicit Linux pinned lab runtime, `test-faults` | 8 admission + 4 binary tests pass; final binary suite 17.84 s, including six worker-kill checkpoints and cancellation |
| Daemon wallpaper CLI integration | 1 test passes; no owner configuration initialized |
| Linux workspace all-target clippy, `-D warnings` | Pass |
| Importer all-target `test-faults` clippy | Pass |
| Windows/macOS cross all-target clippy | Pass for input, proto, ctl, app, tray, wall, wall-sync and wall-import; not a full-workspace cross-check or native execution |
| Importer cargo-deny 0.20.2 | Licences, advisories, bans and sources pass |
| Artifact gate | Cargo + default helper + 32 lab-runtime files pass; 134-component CycloneDX dry-run; injected FFmpeg/GPL and fault-enabled-helper negative checks reject |
| Full daemon unit suite | **Not green**: best short-private-path Linux attempt 357 pass / 8 fail / 2 ignored |

The daemon failures are in unchanged hooks/settings permission checks, private
D-Bus tests, hook execution and the invite slowloris test. Earlier attempts
also hit unavailable socket permissions and long UNIX-socket paths. A further
synthetic private-root attempt stalled in the unchanged update kill/download
test and was terminated with its owned PID namespace (exit 137). These are
not claimed proven baseline failures: no clean-baseline comparison was run.
They remain an unmet requested validation criterion, not an importer pass.
No Mutter/GNOME presentation harness or owner-session access was used.

## Hostile self-review and deliberately unmet exit criteria

- **M1b is not complete or shippable.** Linux RLIMIT_AS is a per-process virtual
  limit, not the design's aggregate ≤1 GiB resident/cgroup ceiling. Decoder
  threads and scratch/kernel allocations are not an aggregate-memory proof.
  Cross-directory imports are not globally serialized. Add delegated cgroup
  memory/pids enforcement and adversarial native measurement before shipping.
- No normalized ≤30 fps VP9/AV1 cache or presentation-ready timeline is emitted.
  The retained source is validated but still untrusted; future playback must
  rehash and isolate decoding, never treat this record as a trust bypass.
- **Windows: UNTESTED / unimplemented native sandbox.** Required design: broker
  snapshots source; launch suspended AppContainer with no network capabilities,
  explicit read-only runtime/input ACLs, handle allowlist, private output broker,
  restricted token and nonbreakaway kill-on-close Job with memory/process/CPU
  limits; assign Job before resume; prove denies and kill cleanup on Windows.
  Compile-checked code currently refuses instead of faking these guarantees.
- **macOS: UNTESTED / unimplemented native sandbox.** Required design: signed,
  hardened App Sandbox helper/XPC service with no network/user-file entitlements,
  one broker-provided read-only input capability, bundle-only plugins, bounded
  sandbox-container scratch, rlimits, broker deadline and audited descendant
  teardown. Signing/entitlements and kernel-denial tests need native hardware.
  Compile-checked refusal code is not an implementation of that equivalent.
- Packaging remains gated on signed immutable source-built runtimes, notices,
  LGPL replacement/source-offer test and native runtime closure inspection.
  Runtime manifests cannot prove their own truth. No runtime is shipped here.
- No independent reviewer or native presentation/consent/geometry claim; those
  are separate tasks. Full M1 and release gates stay open.
