# Wave 6 M1 step B — adversarial Codex review

Date: October 31, 2026 (Asia/Amman).

## Scope and disposition

Reviewed `git diff 1689c16..bd47139` (`bd47139`: “Merge wallpaper M1 step B
(real playback in the private GNOME harness)”). Step B is `c73359d`,
`00eb033`, `78cc574`, `2751687`, `40c481d`, and `02fecf5`. The intervening
audit branch merged at `1b71bf4` is context, not a new step-B finding. In
particular, the unrelated history/settings/UI audit changes in the requested
range were not redesigned.

**Three defects fixed:** medium I420 correctness, medium decoder-startup
availability, and low qualification-counter accuracy. No sandbox escape or
renderer-window impersonation was established. This is not a production
playback approval or a new native GNOME qualification.

## Findings and fixes

### B1 — Medium: odd-height I420 uses the wrong plane offsets and frame size

- Evidence: `crates/wall-import/src/play.rs::i420_planes` placed U directly
  after `Y_stride * height`. GStreamer's default I420 allocation rounds the
  **luma height to even**, independently of the chroma stride rounding.
  For 5x3, the old tuple was `(8,4,2,24,32,40)`; the real layout is
  `(8,4,2,32,40,48)`.
- Independently checked the installed `libgstvideo` layout using
  `gst_video_info_set_format(I420, 5, 3)`: size 48, strides 8/4/4, offsets
  0/32/40. This did not initialize a desktop or run an imported decoder.
- Impact: valid odd-height VP9/AV1 output is rejected against the calculated
  frame size, or its chroma is sampled from padding/the wrong plane. The
  retained even-height qualification fixtures do not cover this case.
- Fix: pad the luma rows in the shared plane-layout helper. Both the worker
  and renderer use this helper; no separate layout convention was added.
- Before-fix regression: `video_streams_carry_i420_with_their_colorimetry`
  failed on the tuple above, and the new
  `crop::tests::odd_height_i420_uses_padded_luma_rows` failed its all-white
  5x3 output assertion. Both pass after the fix, including a negative canvas
  origin. The frame-byte expectation is now 48, not 40.

### B2 — Medium: incomplete worker header blocks presenter shutdown

- Evidence: `crates/wall-presenter/src/playback.rs::start` synchronously
  called `StreamHeader::read` on a blocking pipe before returning to the
  command loop. A silent or partially writing decoder could hold this read
  indefinitely. Stop/Shutdown commands cannot interrupt that read;
  `Presenter::drop` joins the presenter thread. The worker's playback mode
  intentionally has no lifetime CPU-seconds limit.
- Impact: malformed media or a compromised sandbox worker can stall this
  laboratory presenter’s local control/teardown path. The Shell has its own
  kill/watchdog paths; this is not a claim that it loses all recovery.
- Fix: poll the header descriptor against a **single absolute three-second
  deadline**, including partial progress and interrupted syscalls. Failure
  follows the existing `Playback` RAII kill/reap path. No new subprocess,
  authority, or lifetime limit on normal looping playback was introduced.
- Before-fix regression: a partial `WSTR` header held open for 150 ms
  returned EOF instead of the required ten-millisecond deadline error.
  Added tests also check that slow progress does not reset the deadline and
  that a valid header and ordinary EOF retain their existing semantics.
- Qualification limit: a source that cannot produce its header within three
  seconds now fails closed, even though the native preroll operation itself
  permits longer. This is intentional for the lab's five-second first-frame
  watchdog, not a measured production decoder-startup SLA.

### B3 — Low: qualification invents exact-window counters without a baseline

- Evidence: `docs/spikes/gnome/qualify-m1-playback.py::measure` used `{}` when
  no initial loop seam existed, then subtracted a default zero. The archived
  1080p result consequently reports `presented_in_window: 961`, although 961
  is the latest **since-start seam counter**, not a count at the exact end of
  the CPU window. Even two available seam snapshots are not exact wall-time
  window boundaries. Late/starved fields have the same scope issue.
- Fix: read the initial seam list once, emit explicitly named
  `*_between_seam_samples` values, return null without both endpoints, and
  retain raw before/after CPU counters, HZ, and precise elapsed time in future
  results. Historical `result.json` and logs remain unchanged.
- Before-fix regressions: the missing-baseline test found the old misleading
  field and the existing-baseline test could not find the explicit sampled
  field. Both pass now; baseline 100 and final 961 yield 861, not 961.
  `test_m1_playback_measure.py` extracts the actual measurement function using
  Python AST and exercises it with a fake clock and temporary controls; it
  does not import/launch the GNOME harness.
- Also narrowed the report/evidence wording for optional CPU delegation,
  sampled PSNR/red detection, and commit-time versus display-time seams.

## Security and correctness boundaries checked

### Admission, worker, and resources

- Still PNG is one admitted frame with zero animation duration; animation
  timelines remain separately checked. Per-source 512 MiB, dimensions
  4096x2160, 36,000 frames, 600-second total duration, and the billion-pixel
  cumulative decode budget bound the admitted work. Image decoding also has
  its allocation limit. Zero/short image delays and oversized delays are
  rejected rather than converted into spin loops; WebM timestamp differences
  and timeline sums are checked. Stream-header limits are rechecked before
  allocating output frames. No new reachable integer overflow was found in
  the admitted dimensions/timeline domain.
- Playback re-admits media in the worker, with no shell-command construction
  from media. Bubblewrap unshares the worker, exposes the pinned read-only
  runtime and source descriptor, and provides bounded temporary storage.
  Seccomp/no-new-privileges and the subprocess-denial stage preserve decoder
  threads without granting sockets or an external ffmpeg path.
- A private delegation is mandatory for memory/pids containment: 1 GiB,
  no swap, 64 tasks; cleanup owns its child groups. Playback uses
  `cpu.max = 200000 100000` **only if CPU is delegated**. It is a two-core
  equivalent bandwidth quota, not CPU affinity. Memory+pids-only delegation
  is accepted and has no aggregate CPU quota. That behavior is explicit in
  code; corrected the unconditional documentation claim rather than changing
  deployment policy. Continuous playback intentionally lacks a total CPU-time
  deadline. Native cgroup pressure/cleanup was not rerun here.

### Presenter

- Resolves the authorized hash, bounds and hashes the opened source, rewinds
  it, and passes that same descriptor to the sandbox. A concurrent same-user
  writer can still mutate the underlying inode; read-only mounting is not an
  immutable snapshot. This is already outside the stated same-user threat
  model, not a newly fixed TOCTOU guarantee.
- Header format/layout, dimensions and frame count are validated against the
  record before presentation; bounded header parsing checks durations. Exact
  frame reads and validated sizes prevent truncated/untrusted worker pixels
  from becoming arbitrary slice lengths. Worker errors are propagated rather
  than unwrapped in the production frame-read path. B1 fixes the native
  odd-height layout disagreement; B2 bounds header waiting.
- Cover mapping uses rational i128 centre sampling, keeps the reserved canvas
  origin in the calculation, and maps each local monitor slice separately.
  Existing crop goldens exercise centre cropping, negative offsets, gaps and
  plan mismatch. The upstream presentation gate rejects zero/invalid canvas
  rectangles, out-of-canvas slices, excessive edges, and monitor mismatch.
  `Layout::new`/compose are not standalone hostile-plan validators: callers
  must preserve that gate. No admitted-plan zero division or out-of-bounds
  mapping was established.
- Two shared-memory buffers, checked surface/allocation sizes, owned backing
  descriptors, mapping teardown and compositor release tracking were checked.
  Ready-frame and recycle queues are bounded. **Not every queue is bounded:**
  presenter commands and reports use unbounded channels. Repeated commands
  from the trusted local authority could accumulate during slow startup;
  there is no worker-to-command injection route. Recorded as a remaining
  trusted-control robustness limit, not a demonstrated media exploit. The
  bounded header wait reduces, but does not redesign, that concern.

### Extension 78cc574 and release refusal

- `_adopt` checks `Meta.WaylandClient.owns_window(window)` before changing
  type, connecting damage handlers or adopting the actor. Titles/app IDs are
  not authorization. A different client cannot gain ownership by submitting
  a 0x0 buffer. A compromised renderer already owns its authorized connection;
  the geometry exception does not grant another connection that identity.
- Empty geometry returns **before** updating `lastDamage`, setting `fresh`,
  or cancelling/rearming timers. Wrong nonempty geometry still stops the
  renderer. The initial first-frame timeout and subsequent stale-damage
  watchdog remain in force; empty damage cannot keep them alive. This is
  source review, not a fresh Mutter integration test.
- Exercised the artifact self-test, real normal/lab Cargo metadata and
  renderer feature trees, and both optimized-build rejection checks. Normal
  graphs pass; lab metadata and renderer importer dependencies are rejected;
  both ordinary release checks fail with the intended laboratory-feature
  diagnostic. The metadata check examines activated lab features even outside
  the chosen importer root.
- **Provenance limit, not fixed:** the artifact gate consumes supplied
  graph/tree text, not the packaged executable. Substituting an unrelated
  clean tree passes the text check; `package-wallpaper-gnome.sh` accepts a
  caller-selected `BIN_DIR` without binding binaries to that graph. Likewise,
  the compile guard tests debug assertions, not a cryptographic release
  identity. A builder controlling inputs can evade such policy checks. The
  existing controlled build workflow/dry-run must remain the trust boundary;
  no binary attestation or malicious-packager resistance is claimed. This
  predates the step-B playback gate and needs a packaging/provenance task,
  not a local playback redesign.

## What the retained qualification evidence supports

- `result.json` records 100 results with distinct cycle indices 0..99: GIF
  and APNG 15 each; WebP/still/VP9/AV1/1080p clip 14 each. Revoke, disable,
  stop, SIGTERM and SIGKILL each occur 20 times. The harness checks the cycle
  screenshot, exit, worker teardown and restored background; retained result
  summaries report success and empty remaining-process/group maps. These are
  **100 mixed-format lifecycle cycles**, not 100 cycles per format or proof
  of every step-C lifecycle case.
- Exact-output checking for GIF/APNG/still covers all pixels **below the
  excluded 64-pixel panel**, on both virtual monitors. WebP permits a one-level
  channel error: it is not pixel-exact. VP9/AV1 PSNR is calculated at sampled
  source-pixel/block centres, not every displayed output pixel. The script
  threshold is 30 dB; the retained results report at least 34 dB. Red-dominant
  absence is sampled on a 384x101 grid, not a full-frame absence proof.
- The screenshot oracle duplicates the Rust cover formula and thus has a
  common-mode risk. The separate crop golden tests help, but are not an
  independent proof of all compositor scaling/color behavior. Retained PNGs
  are selected quarter-size previews, not all original full-resolution shots.
  Sources/runtime/binaries are identified by hashes, not retained in full.
  Thus the full pixel-exact claims and all 100 screenshot checks cannot be
  independently recomputed from the archived previews alone.
- The 28 seam observations measure renderer **commit** gaps, reported within
  -0.40..+0.26 ms of nominal. They are not Wayland presentation feedback or
  physical-display timing, nor an hours-long stall/wakeup measurement.
- CPU table: approximately 30 seconds, process `utime+stime` tick deltas/HZ
  for renderer/Shell and cgroup `usage_usec` deltas for workers, each divided
  by elapsed wall time. Values are percent of **one core**: 13.1 renderer,
  42.6 worker, 8.5 Shell. The archived JSON lacks raw CPU endpoints, so these
  percentages cannot be independently recomputed; future output retains them.
- Memory table: 30 once-per-second RSS samples, maxima/means as labeled;
  renderer RSS 38.8 MiB (anonymous 17.7, shared-memory 18.5), worker summed
  RSS 80.2 MiB, sampled cgroup current max 66.8 MiB and lifetime peak 67.0 MiB.
  Process RSS can double-count shared pages; it is not additive with cgroup
  memory. The kernel peak need not originate inside the CPU window. There
  was a recorded two-core quota and zero throttled-period delta. B3 explains
  why 961 is a since-start seam count, not an exact 30-second frame count.
- The archive records zero new warnings against its normalized known-warning
  baseline, not zero Shell warnings of any kind. It documents historical use
  of a child inside the agents slice. **This review did not repeat that setup**
  or certify its compliance with the stricter delegation rule in this brief.

## Validation and safety

All Rust gates ran with `env -i`, throwaway HOME/ONE_MK_HOME, a private 0700
XDG runtime directory, no inherited display/session-bus addresses, and one
worktree `CARGO_TARGET_DIR`. Cargo used `--offline -j 2`; no `cargo fmt` ran.
Local-socket tests required sandbox escalation, not a live desktop session.

| Feature set | `cargo test --workspace` | `cargo clippy --workspace --all-targets -- -D warnings` |
| --- | --- | --- |
| default | PASS | PASS |
| `one-mk-wall/gnome-bridge` | PASS | PASS |
| `one-mk/wallpaper-m1-lab,one-mk-wall/m1-lab` | PASS | PASS |

Additional PASS: artifact self-test; real metadata/tree accept/reject checks;
`scripts/test-wallpaper-release-gates.py` (both packages);
`docs/spikes/gnome/test_m1_gate.py` (6 tests);
`docs/spikes/gnome/test_m1_playback_measure.py` (2 tests); `git diff --check`.
Native/sandbox qualification tests that require an explicit runtime/delegation
remain ignored; passing unit gates is not a substitute for them.

An initial run used an unsuitable long/group-writable scratch location and
failed environment-sensitive tests. It was discarded and the complete matrix
rerun in the short private temporary tree. One intermediate default test also
caught the stale 40-byte expectation in B1's regression; the corrected default
suite and both feature suites passed. Neither is hidden as a product failure.

No suitable privately delegated cgroup was supplied to this review. No GNOME
harness, native playback-pressure run, owner service-manager/cgroup operation,
live display, clipboard operation, or real session-bus access was performed.
Review-created target/scratch artifacts are removed at handoff; validation
processes are allowed to finish before cleanup.
