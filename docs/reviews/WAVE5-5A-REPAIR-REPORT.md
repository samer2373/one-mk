# Wave 5A hostile-review repair report

2026-10-30 · branch `t3/fix-clipboard-history-review` · baseline `wave5`
(`5b1cd2d`, “Save the Claude reviews of the Codex wave 5 work”).
Scope: Part A only of `WAVE5-CLAUDE-REVIEW-5A-5F.md`. No 5F changes.

## Finding dispositions

| Finding | Disposition | Repair / evidence |
| --- | --- | --- |
| A1 HIGH privacy | Fixed within stated platform limits | Real native X11/Windows/macOS sensitivity checks; Wayland data-control KDE MIME hint query; X11 runtime regression, other OS paths compile-only. CLI/window lists masked independently; explicit Show/Hide. Default history OFF, including old settings without an explicit choice; one-time clipboard-summary/status upgrade explanation, persistent window OFF state. Explicit retention preserved. Unmarked secrets are not claimed safe. |
| A2 skipped-app sharing | Fixed | Skipped app contents are digest-scoped concealed data for nonexplicit reads (fetch/pointer-follow included), not just history. Both event-time and read-time X11 app checks. Memory regression plus real-Xvfb synthetic WM_CLASS exclusion. Explicit send remains an intentional sharing override, never a history override. |
| A3 update shortcut collision | Fixed | Added defaults yield to existing/custom chords before duplicate validation; both Linux and Windows fetch defaults tested, including `home = ctrl+alt+v`. Custom fetch also takes precedence over a broadcast update default. Stable IDs include FetchClipboard. |
| A4 sticky global flag | Fixed | Removed global history_skip flag; bounded digest exclusions no longer contaminate independent peer writes. Regression switches from skipped local content to a different received item. |
| A5 restore re-recording | Fixed | Successful writes retain encoded cache/readback digest and origin; own selection notifications do not append a duplicate. Real arboard/XFixes regression on private Xvfb, not only the memory backend. Binary window mouse/Enter restore also passes. |
| A6 blocking saves | Fixed for saves/status | Dedicated lazy condition-variable saver; cheap Arc snapshots, bounded 150 ms debounce, single coalesced pending snapshot, unchanged-write suppression, atomic status. Real loopback key forwarding/status measured during a 256 MiB write: snapshot 126.692 µs, maximum sample 3.041661 ms. Startup loads remain synchronous; no append-only journal. |
| A7 metadata-only claim | Fixed | JSON and text list content previews are replaced by a placeholder. Corrected README and QA claims; explicit Show is the only content-revealing list workflow. Masked-list regression and harness assertions. |
| A8 persistence error latch | Fixed | Separate protected corrupt-load state from retryable async save errors. Next change/configuration retries saves/removals. Disable clears load error; Clear recovers corruption without silently overwriting it. Transient-failure and disable regressions. |
| A9 invisible preview characters | Fixed | Unicode category filtering plus default-ignorable tags, variation selectors and fillers; explicit hostile regression set and independent all-scalar category assertions. CLI JSON escapes supplementary scalars correctly with surrogate pairs. |
| A10 shrinking sharing limit | Fixed | Retention item cap separated from sharing max_mb. Regression retains a 2 MiB item at a 1 MiB sharing limit, refuses restore with actionable error, then restores after raising the limit. |
| A11 empty file | Fixed | Empty/disabled snapshots remove saved history. Regressions cover missing, existing valid-empty and corrupt containers; OFF removes corrupt containers without loading them. |
| A12 window list cap | Fixed | Limit derived from 1000 maximal rows rather than 512 KiB. Maximal four-byte-Unicode metadata regression fails under old cap. |
| A13 misleading UI states | Fixed | Explicit disabled/empty/offline/loading/error and memory/disk retention wording. Clear needs a second confirmation click; Refresh cancels it. Private-Xvfb screenshots visually inspected for all these states. |
| A14 fixed-peer fetch | Consciously not changed; documented | Deterministic next ONLINE peer in name order is retained, not round-robin or fallback on refusal. Automatic fallback could silently retrieve from an unintended source. README/QA now say this explicitly and give `clip-get --from NAME`. |
| A15 gaps / footprint | Fixed for identified gaps; runtime coverage limited | Stable action IDs, collision/native-X11/restore/persistence/maximum-parser regressions; default OFF avoids ordinary local image encoding for history. Opt-in default cap reduced 64 → 16 MiB, not advertised as an RSS ceiling. Latest closed-window OFF/ON/OFF samples: 0/0/1 process CPU ticks per five seconds, listener ticks zero throughout, VmRSS 35,812 kB for these small fixtures. An earlier run had 0/0/0 process ticks. |

## Validation and test discipline

- Clean-environment `cargo test -j 2 --workspace`: passes without exclusions;
  the formerly reported portal reconnect test passes in this run.
- Clean-environment `cargo clippy -j 2 --workspace --all-targets -- -D warnings`:
  passes. No formatter was run.
- Requested Windows/macOS package sets, INCLUDING clipboard, pass cross-clippy
  with `--features blake3/pure`. Default-feature attempts hit missing host
  assemblers/Apple C compiler; no toolchain limitation is presented as an
  ordinary default-feature pass. Neither OS was runtime-tested.
- Two ignored native-X11 regressions pass with `ONE_MK_PRIVATE_X11=1` on :197.
  Ignored maximum-save benchmark passes with actual write overlap.
- Updated `scripts/test-clipboard-history.py` passes end to end using its own
  paired daemons/displays/homes, including interrupted save, corruption,
  reconnect, mouse/keyboard actions, excluded-app fetch/pointer-follow refusal
  and idle samples.
- Screenshots inspected: normal, narrow, 200%, revealed, disabled, empty,
  loading, unresponsive, persisted and offline. All synthetic, private Xvfb.
  They show masked defaults, a single explicitly revealed row, clear retention
  warnings and readable state messages. Files remain ignored test artifacts.
- `git diff --check` passes. No owner process/session/config/clipboard was used.
  Own harness children are cleaned in finally blocks; the standalone private
  Xvfb is terminated after validation.

Red-before-fix executions were captured for the initial privacy/restore,
shortcut, sanitizer, persistence and list-cap regressions. Self-review also
added red tests for custom-fetch/default-broadcast collision and valid-empty/
corrupt-container OFF handling, then fixed them. Not every documentation or
measurement finding has an independently executable before-fix test, and some
supplementary coverage was added after its grouped implementation. This is
not represented as literal one-red-run-per-finding compliance.

## Hostile self-review and remaining release risks

Self-review caught and repaired default insertion-order precedence, a valid
empty saved-file corner case, corrupt-file OFF handling, and accidental X11
dependence in native-Wayland reads. Windows/macOS marker checks also reject
clipboard-generation changes across a read. Persistence errors and shutdown
ownership were reviewed for stale snapshots, removal ordering and idle wakes.

Residual limits are explicit: best-effort X11 focus/source attribution and
clipboard races; no detection of unmarked secrets; native Wayland and
Windows/macOS runtime untested; Windows/macOS lack per-copy monitoring;
Windows lacks fetch toasts. Persistence is unencrypted, debounced, not a secure
erase or memory-zeroization feature. Async snapshots/bitmap buffers add RSS
beyond retained-byte caps. Startup load can still delay startup/reconfiguration.
Real global shortcuts, native notifications, real suspend, screen readers,
non-Linux ACL/crash durability and compositor failure modes need hardware QA.

There is no assertion of enterprise certification, no-known-bugs perfection,
or mathematical zero CPU for the entire network daemon. See
`../CLIPBOARD-HISTORY-QA.md` for the contract and repeatable release checks.
