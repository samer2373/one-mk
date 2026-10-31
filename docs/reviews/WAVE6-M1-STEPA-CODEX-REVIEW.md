# Codex adversarial review: wallpaper M1 step A

Date: October 31, 2026 (Asia/Amman).

## Scope and disposition

Reviewed exactly `git diff ff0d44b..2b5bf48`, including the authenticated session,
daemon authority, renderer integration, gate/codec behavior, artifact checker,
qualification harness and checked-in evidence. Adjacent unchanged code was read
only to establish call paths, dependency behavior and release packaging. No
subagents, live GNOME session, X server, clipboard or real session bus were used.

**Three findings fixed**, with failing-before/passing-after regressions. Step A
remains an intent-only laboratory implementation, not authorization for production
playback. The historical GNOME evidence was audited, not regenerated for these
fixes. Findings and limits below supersede stronger interpretations of that run.

## F1 — High: coalesced policy changes can hide revocation

**Evidence:** `wallpaper_authority.rs` originally published only
`watch::Sender<Option<u64>>`. After an admitted revision 9, an applied disable
followed by re-enable leaves the latest watch value `Some(9)`. An already-pinned
renderer fetching after both changes receives its original enabled revision and
the same plan; the session's idempotent path keeps it active. Restoring an earlier
revision after replacement has the same problem. Empty `Changed` signals cannot
recover a transition that is no longer represented in authoritative state.

**Reproduction:** added
`disable_and_reenable_cannot_hide_revocation_from_a_pinned_renderer`, first with
the original update semantics factored into a local `Policy` constructor/method.
It failed with `left: Some(9), right: Some(9)` after the two applied changes.
This is an authorization-lifecycle defect even though current activity is only
logged intent and the feature is laboratory-only.

**Fix:** `74c1bac` retains a monotonic internal consent epoch and pins it alongside
the renderer connection. Any intervening applied policy change makes that
connection receive a signed disabled reply, including when changes coalesce.
A new connection can use the newest grant. Unchanged or unrelated policy does
not invalidate a session; epoch exhaustion permanently fails closed until
authority restart. No wire format or presentation design changed. Five authority
unit tests pass, including restoration and epoch-exhaustion checks.

## F2 — Medium: laboratory release exclusion was not enforced

**Evidence:** both original feature entry points were gated only on Cargo features,
not build mode. Clean-environment `cargo check --release -j 2` succeeded with
`-p one-mk --features wallpaper-m1-lab` and independently with
`-p one-mk-wall --features m1-lab`. The new release regression initially failed
because the daemon laboratory feature compiled successfully.

The existing checker did reject the authority dependency in a correctly supplied
daemon feature tree. However, its Cargo closure check inspected only the requested
package (normally the importer). A fixture containing an activated laboratory
feature outside that closure passed despite a clean daemon feature-tree input.
The regression failed with `activated laboratory feature escaped metadata gate:
one-mk/wallpaper-m1-lab`; the presenter feature was likewise not checked.

**Fix:** `f266b5e` adds compile-time guards when debug assertions are disabled
(including the normal release profile), checks activated daemon and presenter
laboratory features across the supplied resolved Cargo metadata, and adds CI
coverage. `scripts/test-wallpaper-release-gates.py` now requires the specific
compiler refusal for both packages; missing dependencies or unrelated compiler
failures do not count as success. Artifact-checker self-tests reject both
metadata injections.

**Limit:** this checker consumes build metadata and a feature tree; it does not
attest a binary. Supplying a clean graph for a different artifact is still invalid
input that the tool cannot detect. Deliberately enabling debug assertions in a
custom optimized profile also changes the guard's premise. Default release
workflows do not enable the laboratory features; arbitrary modified build
pipelines are not a security boundary.

## F3 — Medium: qualification failure paths skip private-process teardown

**Evidence:** `qualify-m1-presentation.py` originally set `started = True` only
after `isolated-session.sh start` returned successfully. That script starts a
private process group/buses before waiting for the Shell socket, and can fail or
time out after allocating them. The Python finalizer then skips `stop`. Also,
copying `shell.log` preceded `stop` in the same finalizer: a copy error skipped
teardown even after a successful start.

**Reproduction:** `test_presentation_cleanup.py` replaces all process-launch
operations with a small allocated-session model; no GNOME or D-Bus is launched.
The original ordering failed both start-error and start-timeout cases. After
fixing those, a third evidence-copy-error case independently failed with
`failed start left its private process group running`.

**Fix:** `51fc108` marks the new private session for cleanup before attempting
startup; `f2351d2` runs stop in a nested `finally` even if log copying fails.
The regression passes all three failure cases and is included in CI. This fixes
the new qualification wrapper, not every possible failure of the underlying
external process supervisor; a supervisor that itself cannot stop requires
separate recovery. Historical successful-run cleanup does not test these errors.

## Other security/correctness checks

- **Tags and nonces:** keyed BLAKE3 has distinct request/reply domains and
  length-delimited fields. Requests bind both unique names, nonce, serial and
  monitor tuples; replies bind both names, nonce, enabled/revision and exact
  payload. The renderer generates a fresh 32-byte nonce for each actual fetch;
  randomness failure closes the session. The pinned authority PID/UID are queried
  from the bus after subscribing to name loss. The application cannot choose or
  squat another connection's unique sender name. No cross-request replay or
  reflection bypass was found under this transport contract.
- **Comparison and key lifetime:** inspected the pinned BLAKE3 implementation:
  `Hash` equality uses `constant_time_eq_32`; rejecting incorrect public tag
  lengths early does not disclose key material. Fresh daemon keys/generations
  are not reused on restart and debug output redacts the key. Keys are not
  explicitly zeroized, and SIGKILL can leave a private stale record on disk.
  Its old unique name/key do not authorize a replacement daemon. Same-UID memory
  access, record readers and a compromised bus are outside the documented model;
  this review does not claim protection against them or core-dump recovery.
- **Files and races:** startup checks private owned runtime directories; renderer
  record reads check the opened descriptor's ownership/type/mode, use leaf
  `O_NOFOLLOW`/nonblocking open and a 513-byte read cap. Daemon publication uses
  exclusive creation, mode 0600 and rename while holding a lock. These are not
  dirfd-based, race-free operations against hostile same-UID directory mutation.
  Trusted runtime ancestry and the explicit same-UID exclusion remain prerequisites.
  PID reuse alone does not satisfy the unique-name and fresh-key checks.
- **Gate/refusals:** authenticated invalid plans retain admitted state and sequence;
  consent/owner loss remains terminal. Identity is checked before live-consent
  reconciliation. Generation, revision, sequence, serial, media, geometry,
  duplicate screen names and overlap are checked before committing a plan. Layout
  changes discard old admission. Identical authenticated payloads are idempotent,
  not sequence rollback. The first-plan failure clears its provisional gate.
  Checked sequence addition and widened rectangle arithmetic prevent wrapping;
  serial exhaustion fails closed rather than accepting a wrapped layout.
- **Geometry/reservations:** the gate freezes the configured canvas, requires
  all local monitors to share one translation and forbids overlap/out-of-canvas
  slices. It permits unoccupied canvas space, so absent-computer slots are not
  collapsed. The authority/configuration is trusted to select that canvas and
  origin; this is not a separate per-peer slot allocator or media verifier.
- **Input/resources:** codec allocations are bounded by payload/count/screen
  limits, and transport envelope checks precede deserialization. No new panic
  reachable from the reviewed uncredentialed wire path was found. The session's
  gate `expect` follows construction or an early return. D-Bus/zbus still receives
  messages up to the bus's own limit before application body rejection; the
  4096-byte codec budget is not a process-memory isolation guarantee. Bootstrap
  descriptors and fault directives are trusted laboratory inputs, not public IPC.
- **Teardown:** owner loss is selected before other session events, fetches have
  a 750-ms deadline, and successful revoke/disable/stop/SIGTERM/SIGKILL traces
  show renderer exit. The 2-second heartbeat is a recovery mechanism, not an
  unconditional 2-second teardown guarantee for every stalled-bus scenario.
  There is no decoder/surface process tree here to prove destroyed or restored.

## Evidence audit

Independently verified all **15 checked-in source-file hashes** in
`m1-presentation-evidence/source-sha256.txt` against `2b5bf48`. The two historical
binary hashes cannot be independently reproduced from the retained text files.
Renderer logs contain exactly one initial admission for each revision 1000–1099,
consistent with the reported 100 cycles. Re-running the warning comparator on
retained Shell/baseline logs gives **14 records, zero new warnings**. The recorded
5–86 ms teardown measurements are supported by the harness's monotonic-clock
measurement code and report, not independently recoverable timing from every
raw log line.

The logs support refused absent/stale/wrong-key/wrong-PID/impostor credentials,
retained plans after injected faults, and steady-state lifecycle tests. They do
not test coalesced policy transitions, failed startup cleanup, actual playback,
surface restoration, hardware/HiDPI, suspend/resume or physical multi-computer
slot ownership. Process cleanup scans match this lab's runtime marker and ignore
unreadable `/proc` entries; an empty result is not universal host-process proof.
No GNOME run was made during this review, and original evidence was not rewritten
or presented as evidence for the fixes.

## Validation and cleanup

All six required final workspace gates passed with `--offline --locked -j 2`:

| Feature selection | Workspace tests | Ignored | Clippy, all targets, `-D warnings` |
| --- | ---: | ---: | --- |
| default | 1,190 passed, 0 failed | 17 | passed |
| `one-mk-wall/gnome-bridge` | 1,191 passed, 0 failed | 17 | passed |
| `one-mk/wallpaper-m1-lab,one-mk-wall/m1-lab` | 1,196 passed, 0 failed | 17 | passed |

Each test run produced 42 successful result records, including doc tests.
Additional checks: seven Python warning/cleanup tests passed (the cleanup test
contains three failure subcases); artifact self-tests passed; both optimized
laboratory builds were refused with the intended diagnostic. With real generated
Cargo metadata and a real default daemon feature tree, the checker accepted the
clean graph (101 closure records) and rejected the feature-enabled graph even
when given that same clean daemon tree. `git diff --check` passed.

Environment: `env -i`, toolchain-only PATH and explicit Cargo/Rustup cache homes,
throwaway HOME/ONE_MK_HOME/TMPDIR and mode-0700 XDG_RUNTIME_DIR. One worktree-local
`CARGO_TARGET_DIR` served every build, including release-negative checks. Initial
attempts hit sandbox-denied loopback binds, then test Unix-socket path limits,
then group-writable scratch ancestry. These were environment failures, not fixed
by unrelated source changes: gates were rerun with approved loopback access and
short, private scratch paths, and all six final gates passed.

Cleanup completed: the final review-marker process scan found no surviving test
processes. The single build target, all worktree review scratch, and the short
`/tmp/omkr.fQCLwf` tree were deleted and their absence checked. No GNOME/harness
process was actually started by this review. No formatter, owner configuration,
live session bus, X server or clipboard was used or modified.
