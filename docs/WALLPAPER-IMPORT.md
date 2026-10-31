# Wallpaper import: M1b development implementation

Security follow-up evidence: **2026-10-31**. Baseline reset: local `wave5`,
commit `c5bdfe7`; implementation branch `t3/wallpaper-import-security`.
This is an optional, finite **source admission and poster** pipeline, not a
wallpaper presenter, normalized cache, consent implementation or M1 sign-off.
The existing presentation stop is unchanged; no Mutter/GNOME harness was run.

## Trust boundary and process lifetime

The shipping daemon does not depend on `one-mk-wall-import` or GStreamer.
It already uses `image` with PNG for clipboard support; it is not decoder-free.
Build shipping binaries separately from the optional importer:
`cargo build --release --locked -j 2 -p one-mk -p one-mk-tray -p one-mk-app`.
An explicit `--workspace` build unifies importer GIF/WebP features into shared
dependencies and is not the shipping build command. The artifact gate checks
`cargo tree --locked -p one-mk -e features`, allowing only PNG image features.
Its explicit `wallpaper` command dispatches before configuration,
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
executable and one read-only input, read-only private proc/dev and a 16 MiB
scratch tmpfs. `/dev` has an explicit 64 KiB size and is remounted read-only;
`/dev/shm` is a read-only empty directory. Only null/zero/urandom device nodes
are bound. Every tmpfs has an explicit size. Doctor checks mount capacity,
read-only flags and actual write denial before advertising availability.
No host home, document tree, display socket, render node or network capability
is mounted. The parent exports architecture-checked seccomp to a sealed memfd
and passes `--seccomp FD` to bubblewrap: it applies before the worker's runtime
loader executes. The worker verifies inherited seccomp and `no_new_privs`.
This is a **denylist, not a default-deny allowlist**. Socket/connect/bind/listen,
accept/accept4, ptrace, process-VM operations, io_uring, keyctl/add_key/request_key,
bpf, perf_event_open, userfaultfd, open_by_handle_at, mount/umount and the new
fsopen/fsmount/move_mount/open_tree/mount_setattr/fspick APIs are denied.
Unshare/setns, namespace clone flags (including NEWTIME), and alternate x32
entry are blocked; clone3 returns ENOSYS. Bubblewrap also disables nested user
namespaces and asserts that restriction. Memfd creation and SysV shmget/shmat
are denied, so anonymous shmem cannot bypass the scratch-file ceiling.
Explicit MAP_HUGETLB mappings are also denied and checked by doctor, rather
than relying on optional hugetlb accounting in a particular kernel/mount.
Only anonymous AF_UNIX socketpairs remain for
GStreamer internal wakeups; no external endpoint can be opened.

After admission, image workers install an additional process-creation filter.
WebM decoding creates one fixed
`/worker --decode-webm` wrapper, which installs restrictions before executing
the pinned gst-launch pipeline; subsequent process creation is denied (thread
clones remain allowed). **Execve/execveat are not blocked**: replacement of the
current process remains possible. Before that additional filter, ordinary
fork/clone is allowed for the fixed wrapper. A compromised loader therefore
has more authority than a post-admission decoder; this is not arbitrary-code
containment by an allowlist. There is no decodebin, URI loader or encoder in that
pipeline. GStreamer plugin registry/search paths are private and explicit;
registry scanning does not fork. PID namespace teardown and bubblewrap's
parent-death contract kill descendants on worker/launcher death. The launcher
also gets PR_SET_PDEATHSIG with a parent-PID race check, covering importer
SIGKILL. The parent
drains bounded output/error pipes, kills on timeout/cancel/overflow, and waits
for both child and reader threads. SIGINT/SIGTERM cancellation returns 130.

**Fallback is refusal.** No ordinary-process, host-GStreamer, FFmpeg, privileged
daemon or unsandboxed import fallback exists. Namespace/seccomp/runtime setup
failure produces `sandbox unavailable: refusing to import`. A Linux platform
without this bubblewrap configuration is unavailable, even if another sandbox
technology might be installed.

**Resource slice (2026-10-31): no weaker memory fallback.** Both doctor and import
now require an explicit `--cgroup-root` on Linux. An administrator/lab supervisor
must separately provide an empty delegated cgroup v2 domain with `memory` and
`pids` already enabled in `cgroup.subtree_control`. The importer never enables
controllers, moves the parent, calls a service manager, or discovers/mutates the
owner's session hierarchy. A missing/read-only/invalid delegation refuses before
worker spawn. Each worker gets a fresh child cgroup with read-back-verified
`memory.max=1073741824`, `memory.swap.max=0`, `memory.oom.group=1`, `pids.max=64`.
The launcher writes `0` to its pinned `cgroup.procs` descriptor in pre-exec,
before bubblewrap or any untrusted runtime loader runs. All descendant processes
and threads inherit the group; the sandbox cannot see the cgroup filesystem.
The existing per-process rlimits remain additional limits, not the aggregate cap.

Root path components and controls are opened without following symlinks, and
the root must be on cgroup v2 with trusted ownership/modes. A nonblocking root
directory flock serializes workers sharing that delegation, including imports
with different output directories; separate delegations are not a machine-wide
queue. The explicit root is a trusted administrator input, not media metadata.
This is a ceiling on cgroup-charged memory, not an assertion that parent snapshot
page cache, inherited/shared charges or GPU allocations are included.

After success, cancellation, timeout or worker failure, `cgroup.kill` tears down
the entire group before pipe joins, and bounded `cgroup.events` polling waits for
empty membership before removing it. Cleanup failure refuses the import. Pipe
readers are nonblocking and stop after teardown even if a leaked descendant
holds a writer open. A parent SIGKILL still relies on the existing bubblewrap
parent-death/PID-namespace contract and can leave an empty cgroup; there is no
automatic stale-cgroup deletion or authorization to kill a pre-existing group.

**Not a kernel-pressure qualification:** this implementation environment mounts
cgroup v2 read-only. Controller write/readback, real OOM/task pressure, concurrent
native worker teardown, and parent-SIGKILL cleanup have not been demonstrated
with this new path. The explicit-delegation kernel integration test is ignored
unless deliberately selected in a private laboratory. Do not ship based on the
unit tests alone. See [slice report](reviews/WAVE6-M1-REPORT.md).
The bounded scratch pressure test verifies
mount/rlimit caps first, then writes at most 16 MiB plus one 64 KiB attempt;
it never attempts to fill host memory. Device writes do not allocate storage;
read-only mounts and shmem API denial are checked separately.

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
| Total decoded work | at most 1,000,000,000 pixels, rejected at admission before decode; reduce resolution/frame count/duration and convert first |
| Image decoder allocation | explicit RGBA canvas reservation and image::Limits 256 MiB best-effort allocation policy; frames streamed/disposed; not an aggregate or strict codec-internal ceiling |
| Uncompressed poster | opaque black alpha flattening, at most 320 × 180 RGB PPM |
| Metadata | at most 256 KiB before passing to decoders |
| Per-process virtual memory | RLIMIT_AS 1 GiB, inherited by decoder |
| Aggregate worker memory / tasks | mandatory cgroup v2: 1 GiB charged memory, zero swap, 64 tasks (threads included); native pressure acceptance pending |
| Per-process CPU / wall time | 120 CPU seconds / 600 wall seconds (doctor: 5 seconds) |
| File / descriptors / cores | 512 MiB per file, 64 descriptors, no core dumps |
| Worker result / stderr | 1 MiB / 16 KiB; overflow tears down worker |

Timeline limits are independent upper bounds, not a promise of ten minutes at
4K: the pixel budget permits 113 frames at 4096 × 2160 or 482 at 1920 × 1080
(about 1.88 s and 8.03 s at 60 fps). Small canvases can reach 36,000 frames /
ten minutes. Boundary tests reach independent frame, duration, fps and exact
pixel ceilings without large allocations.

The source and poster are rehashed in the parent. Strict bounded worker JSON
must have valid format/dimensions/frame/duration/fps/pixel-budget fields, checked
again in the parent. The parent requires exact canonical P6 dimensions,
maxval 255 and exact RGB payload length for the expected aspect-preserving
thumbnail (no upscaling). A P6 prefix alone is insufficient. Source, poster and
record use create-new files (0600) in a 0700 directory; fsync precedes Linux
`renameat2(RENAME_NOREPLACE)`, then output-directory fsync. Content directories
are named by source BLAKE3. Existing objects, including symlinks, are never
overwritten. `record.json` contains schema, source/poster hashes, source size,
format/dimensions/frame count/duration and a fixed qualification warning. It
contains no timestamps, absolute source path or owner metadata; repeating an
import into an empty output produces the same record and poster hash.

Failure before commit removes staging through RAII. A parent SIGKILL can leave
an unpublished `.import-*` directory; owned private partials are cleaned
immediately under the exclusive output lock at the next explicit import,
never by an off-state timer (bounded
to the first 4,096 directory entries per activation). Completed objects survive
failure. A directory-sync error after rename explicitly reports that commit
already happened; never blindly retry as though nothing persisted.

## CLI contract

```text
one-mk wallpaper import FILE --runtime AUDITED_ROOT --output PRIVATE_DIR --cgroup-root DELEGATED_DIR [--json]
one-mk wallpaper doctor [--runtime AUDITED_ROOT] [--cgroup-root DELEGATED_DIR] [--json]
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
group/world-writable runtime members are refused. Every ancestor and member
must be owned by root or the caller. Directory components are opened with
O_PATH/O_DIRECTORY/O_NOFOLLOW and checked by descriptor; the exact checked
root is held through bubblewrap `--ro-bind-fd`, not resolved again by path.
This pins directory identity, not immutable contents. Runtime identity is an
**administrator/packager trust input**, not something obtained from the media.
The manifest is not signed: it does not authenticate a malicious packager or
prevent same-user concurrent mutation. A production immutable signed package
and provenance audit remain required.

`scripts/check-wallpaper-artifact.py` traverses the importer Cargo closure,
checks SPDX AND/OR/WITH expressions, and delegates runtime validation to the
helper's `--runtime-inventory` command. Import and packaging share the same
Rust policy: bounded ELF64 little-endian parsing checks NEEDED closure,
rejects RPATH/RUNPATH and ELF filter/auxiliary loader edges, and requires exactly these plugins:
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

The Python self-test injects an FFmpeg Cargo node and asserts the exact expected
rejection (a missing root/graph no longer counts as success). It also tests GPL,
unknown/malformed SPDX and forbidden daemon features. Rust runtime tests reject
an inventoried extra OpenH264 plugin and unbundled ELF edges/loader paths.
`--helper`
executes the trusted build's `--build-info` and rejects fault-injection builds.
Use this explicit invocation on a candidate runtime before any future packaging:

```sh
cargo metadata --locked --format-version 1 > target/wallpaper-metadata.json
cargo tree --locked -p one-mk -e features > target/daemon-features.txt
python3 scripts/check-wallpaper-artifact.py --metadata target/wallpaper-metadata.json \
  --daemon-features target/daemon-features.txt \
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

The following retained lab commands/results predate the mandatory cgroup slice.
Current doctor/import/storage-check commands additionally need `--cgroup-root`
pointing to an explicitly provisioned private delegation. The native binary test
helper adds that option from `ONE_MK_WALL_TEST_CGROUP_ROOT`. Neither a historical
successful import nor an unconfigured conditional-test `ok` qualifies the new
resource boundary; see [current validation](reviews/WAVE6-M1-REPORT.md).

Use a cleared environment with explicit Cargo/rustup homes; `cargo ... -j 2`;
never run cargo fmt. Generated media/tools belong under ignored
`target/wall-import-lab`, not the small host `/tmp` filesystem. Runtime ownership
and permissions must be trusted along the entire absolute path. If the worktree
has a writable ancestor, use a disposable directory below a caller-owned 0700
trusted path; do not relax runtime policy for the lab. Remove it after testing.

`scripts/wallpaper-lab-runtime.py ROOT --worker BINARY` creates a **lab-only**
snapshot from a matching trusted installed GStreamer. It is not a redistributable
runtime, supply-chain pin, licence audit, source offer or update mechanism.
Run `--help` for its exact arguments. Generate image fixtures using the crate's
`fixtures` example. The explicit sandbox test also expects two original
small six-frame, 30 fps clips at `media/six.webm` (VP9) and
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
via SIGTERM. It also SIGKILLs the import parent, verifies an unpublished partial,
then immediately retries and verifies recovery. An RAII child/subreaper guard
reaps tracked descendants even if an assertion fails. It verifies empty output
and disappearance of recorded descendants. A separate bounded storage test
checks denial on read-only mounts and ENOSPC at the private scratch cap.
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

### Prior baseline validation record (2026-10-30)

This is historical evidence retained from `wave5`, not a claim that the full
daemon suite or native operating systems were requalified here.

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

### Security follow-up validation (2026-10-31)

- Importer: 7 unit, 9 admission/property and 5 real binary tests pass with an
  explicit pinned 32-file Linux lab runtime and `test-faults` (binary suite
  43.55 s). Tiny GIF/APNG/WebP and original 16 × 16 VP9/AV1 six-frame clips
  exercise actual decoders, not throughput. Default tests also pass; opt-out
  real-runtime early returns are not counted as qualification.
- Doctor verifies external seccomp, nested-userns refusal, read-only bounded
  `/dev`, bounded scratch and rlimits. The capped write test reaches ENOSPC.
  No unbounded allocation or host-memory-fill test was run.
- Linux whole-workspace/all-target/all-feature clippy passes with `-D warnings`.
  Windows x86_64 and macOS arm64 all-target/all-feature cross-clippy passes for
  input/proto/ctl/app/tray/wall/wall-sync/**wall-import**, with `blake3/pure`.
  This is compile-only, not native execution or full-workspace cross validation.
  Full-workspace attempts stop in `ring`: Windows lacks MSVC `lib.exe`; macOS
  lacks a target-capable C compiler/SDK (`cc` rejects `-arch`/deployment flags).
- Wallpaper daemon unit tests (4), clean/null-backend CLI (1) and e2e tests (3)
  pass, including distinct replay/identity/epoch/rate-limit reasons and actual
  idle CPU tick measurement. Fabricated timers/children status fields are gone.
  The full daemon unit suite was not rerun; historical failures remain open.
- Shared runtime inventory + Cargo/helper/daemon-feature gate passes with 134
  CycloneDX dry-run records. Python negative self-tests pass. No artifact ships.
- Cargo-deny 0.20.2: advisories, bans, licences and sources pass; warnings are
  duplicate miniz_oxide versions and unused licence allowances. CI pins this
  version and uses the verified global `--config` argument before `check`.

### Historical per-finding disposition and hostile self-review

The resource slice above supersedes item 1's unimplemented-controller/fallback
statement; its native acceptance remains outstanding. Old reproduction commands
below require `--cgroup-root` with the current helper; the native binary tests
add it from explicit `ONE_MK_WALL_TEST_CGROUP_ROOT` laboratory configuration.

1. **Fixed with documented fallback:** read-only 64 KiB `/dev`, read-only shm,
   explicit 16 MiB scratch, doctor and real capped ENOSPC regression. Hostile
   self-review additionally reproduced then blocked memfd/SysV shmem escape
   from the storage cap. **Consciously not implemented:** delegated cgroup
   aggregate memory/pids enforcement; it needs a controlled delegated hierarchy
   and native qualification, not mutation of the owner's session hierarchy.
   Doctor and this document disclose the weaker development
   fallback. Aggregate-memory production acceptance remains open.
2. **Fixed:** externally installed sealed seccomp, trusted ownership and mode
   across ancestors/members, O_PATH directory pinning and `--ro-bind-fd`, plus
   disabled nested userns. Same-UID runtime-content mutation is still outside
   the trust boundary; immutable signed packaging remains necessary.
3. **Fixed:** pixel admission precedes decoding with conversion guidance;
   independent limits and actual resolution-dependent frame budgets are stated.
   A 114-frame 4K GIF structural regression admitted before the fix and now
   fails at pixel admission. Tiny image tests reach width/allocation limits;
   replacing decoder `set_limits` with defaults made the test fail. The WebP
   allocation test exposed its best-effort max_alloc behavior, motivating an
   explicit canvas reservation. Strict total codec allocation is not claimed.
4. **Documented alternative, not an allowlist:** expanded the denylist to the
   reported key/mount/socket/nested-namespace gaps; documented exec replacement,
   pre-additional-filter process creation and allowed thread clones precisely.
   A tested default-deny policy for every loader/codec version remains future
   work; pretending an incomplete allowlist is safe would be worse.
5. **Fixed:** parent metadata limits and canonical P6 header/payload validation;
   forged poster and fps/pixel regressions failed before the fix.
6. **Fixed via build isolation and truthful claim:** shipping package selection,
   default members excluding importer, explicit separate importer build, and
   a daemon feature-tree gate. Whole-workspace builds still unify features;
   clipboard PNG remains intentionally present.
7. **Fixed scoped gates/lifecycle:** remove invented status fields, distinguish
   flood reasons, require the intended dependency-negative failure, share runtime
   plugin/ELF validation, expand cargo-deny FFmpeg/libav/mpv/GPL codec families,
   and test parent SIGKILL/recovery. The recovery test failed against the old
   24-hour cleanup and now passes immediately under the lock. OpenH264 policy
   and status regressions failed first. Cargo bans are finite names, not a
   provenance oracle; renamed binaries/crates and dishonest licence metadata
   still require supply-chain audit. CI without a runtime is not sandbox proof.

## Deliberately unmet exit criteria

- **M1b is not complete or shippable.** Mandatory delegated cgroup memory/pids
  setup is implemented, but native controller/pressure/teardown qualification
  is not proven. Cross-directory workers sharing a root are serialized, not
  workers in independently supplied delegations. Adversarial native measurement,
  stale-group recovery after parent death, and administrator provisioning remain
  release gates. RLIMIT_AS alone is never advertised as aggregate containment.
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
