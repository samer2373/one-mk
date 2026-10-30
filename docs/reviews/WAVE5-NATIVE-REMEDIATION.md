# 5H native review remediation — 2026-10-31

> Historical first-round results below are superseded by **Owner-only feature
> gate and regression remediation** at the end of this document. In particular,
> the earlier best-effort-up / fail-open-capture policy is no longer used by
> experimental adapters. Default/release binaries exclude native local remap.

Branch: `t3/fix-native-local-remap`. Baseline: local `wave5`,
`c5bdfe7` (Save the Claude review of the native local remap).
Review: `WAVE5-CLAUDE-REVIEW-5H-NATIVE.md`, **all 22 findings**, including its
file/line references, scenarios and proposed fixes. Original review locations
refer to that baseline, not the modified source.

**EXPERIMENTAL, default OFF. Native Windows/macOS COMPILE-CHECKED ONLY / UNTESTED.**
"Corrected" below means a source-level defect addressed with Linux tests and/or
native compile checks, not a native acceptance claim or a no-known-bugs claim.
No desktop hooks/taps were installed or native acceptance scripts run here.

## Finding-by-finding disposition

| # | Disposition | Change and regression evidence |
| --- | --- | --- |
| 1 | Corrected | `windows.rs` handles only power suspend/resume 4/7/18; charger/battery broadcasts cannot end capture. No Local means no added `end`. Native wiring contract checks the filter and off guard; AC/battery acceptance remains NOT RUN. |
| 2 | Corrected | `activate` always reaches the pinned main activation body after a nonwaiting best-effort fence. Off/empty/unsupported removes Local. All six ordinary actions preserve health; shortcut trigger ups never enter Mapper. `ordinary_hotkey_preserves_worker_health`, `shortcut_release_never_enters_the_mapper`, pinned activation contract. |
| 3 | Core liveness corrected; delivery guarantee consciously not claimed | macOS capture no longer depends on a successful receipt/fence; Quit stops its run loop directly and detaches dispatch. Downs have a 100 ms receipt bound; ups are best-effort posts, not delivery acknowledgement. This intentionally gives up a native up-delivery guarantee rather than wedging core KVM. OS calls themselves cannot be bounded by a pure Rust harness. Source contracts pin no-gate activation/quit. Missing receipt and actual release behavior require the disposable Mac script. |
| 4 | Corrected | Windows release closes the local gate unconditionally; off removes Local entirely. Closed-home randomized stream covers repeat/unowned-up pass-through; source contract scopes `gate.close` to `release`. |
| 5 | Corrected | Removed historical `Physical.mapped`; track outstanding correlated source downs, deduplicating repeat. Clear source on up/baseline; an unowned up passes. `unobserved_up_after_mapped_release_is_os_owned`, overlap-device tests and owned-up failure regression. UAC/elevated native acceptance NOT RUN. |
| 6 | Corrected recovery mechanism; OS denial remains possible | Retry releases on unlock/desktop/normal foreground (also resume). A release-only `Recovery` survives off without retaining interception; re-enable alone waits for old cleanup. Baselines clear absent physical owners; Enter is not refused based on injected async state. `failed_release_remains_owned_until_a_later_successful_retry`, source lifecycle contracts. Cannot deliver while UIPI or the OS still refuses injection. |
| 7 | Corrected | Active flags keep only tracked bits present in the original event, with side-specific modifier checks. Paused/off returns the original exactly; baseline resets flags and physical keys. Random disabled flag differential, missed-up and stale opposite-side tests; mutation of the identity path is killed. |
| 8 | Corrected | Only emergency fails admission; ordinary shortcut pump paths no longer pause/fail mapping. Regression iterates Cycle, Jump, Home, LockPointer, Broadcast and FetchClipboard, including owned trigger-up routing. |
| 9 | Corrected | Fence uses `Mapper::set_keymaps` after release-all, retaining swallow tombstones. Owned ups during failed/remote routing still retire mapper/source state. Fence/repeat/dual-role and failure-up tests prevent re-press/spurious tap and sticky ownership. |
| 10 | Mitigated; timing consciously not guaranteed | Added out-of-context foreground notification alongside desktop changes. Elevated foreground pauses/releases; normal return retries. Notification can arrive after UIPI applies, so release-before-elevation cannot be guaranteed. Retained ledger/retry addresses the later return. The elevated foreground case remains in native acceptance; no fake test is claimed to model UIPI. |
| 11 | Corrected profile; panic-release supervisor not added | Restored release `panic = "abort"`; removed worker catch-unwind wrappers and explicit panic/unwrap/expect paths. Source contract rejects these patterns in native adapters. A process-wide release panic hook was not added: safely running native injection/reentrant cleanup during arbitrary panic/lock state is not established. Fail-stop is restored; abort/kill can still strand an already injected key, as documented. |
| 12 | Corrected | Enable baselines held keyboard keys as OS-owned; off never consults a held-key refusal. Held Enter/repeat/up regression then verifies the next mapping is admitted. |
| 13 | Mechanism corrected; native delivery UNTESTED | Replaced NSWorkspace observers with IORegisterForSystemPower attached to the capture run loop. Register only on opt-in, refuse mapping on setup failure, unregister on off/quit. Existing CGSession/Secure Input checks run before injection and while held; no claim of instantaneous idle session notification. |
| 14 | Corrected | Tap disable calls main's re-enable path; it ends capture only when a Local output was held. No Local means no capture end. Native wiring contract checks the conditional, manual timeout case retained. |
| 15 | Corrected | Mouse codes excluded from physical keyboard baseline. Held-Enter/mouse regression proves button-up absence cannot inhibit the next keyboard remap. |
| 16 | Corrected | Failure clears macOS `down`, Flags and `physical_held`; paused keys cannot re-arm the held poll, and failed watchdog health parks. Source contract checks the park/healthy guards. Native idle wakeup measurement NOT RUN. |
| 17 | Corrected | Failed engine fences may advance to a drained watermark and retain mapper tombstones. Healthy fences still cannot skip unprocessed work. Sequence-gap recovery and existing undrained-fence/generation tests enforce both sides. |
| 18 | Mitigated; full order guarantee consciously not implemented | Base-only snapshots bypass device correlation. Pending admissions use a high-resolution 1 ms waitable timer, canceled at idle, rather than relying on WM_TIMER granularity. Device-rule correlation still has its 8 ms window; ineligible keyboard/mouse passthrough may overtake mapped output. Fully serializing all passthrough input would substantially broaden interception and is not claimed safe without native design/acceptance. |
| 19 | Policy corrected | Physical shortcuts only while experimental mapping is attached, on both OSes. Windows suppresses the parallel WM_HOTKEY path during Local lifetime; off restores the original RegisterHotKey behavior. Allocation test now exercises configured physical chords, including matching/release. Native registration/timing parity remains UNTESTED. |
| 20 | Ordinary metadata corrected; deferred mixed-type provenance incomplete | Carry KeyboardEventKeyboardType through `Admission::Key` to Native; held output ups retain the down's type. A mixed-type keyboard interrupting another keyboard's pending dual-role batch can still require per-Emit provenance, which Mapper currently does not expose. That broader protocol change is consciously deferred rather than pretending the simple field fixes every origin. ISO/JIS/mixed keyboard acceptance remains a blocker. |
| 21 | Corrected | Off/empty sends `problem: None`; doctor returns Skip rather than unconditional native-local WARN. Unsupported configured rules still explain refusal. `local_keys_never_claim_native_acceptance` checks silent native absence without claiming acceptance. |
| 22 | Corrected test/doc claims, bounded scope explicit | Document settings/device auto-resume, not explicit-reload-only. Allocation test includes bindings and actual chord paths. Renamed the pure health/fence modifier stream test so it no longer claims to model tap-disable/Secure Input; replaced its vacuous zero-flags assertion with nonzero stale flags. Added pinned main bodies, randomized pass-through/flag contracts and two killed mutations. Native source checks are labeled source checks, not OS simulations. |

## Failing-first and mutation evidence

- Initial regressions failed against the wave5 ownership, hotkey health and
  sequence/fence behavior, then passed after the shared fixes.
- Additional `owned_up_during_failure_retires_the_source_tombstone` failed when
  an up suppressed during failure did not enter the retiring queue. Queueing that
  up fixed it and exposed shortcut trigger releases being sent to the mapper;
  separate shortcut ownership fixed that regression as well.
- Mutation 1: changed disabled `Flags::rewrite` from identity to `original ^ 1`.
  The randomized bit-exact off/paused test failed, not merely compilation.
- Mutation 2: reintroduced a Local-presence refusal in Windows `activate`.
  The pinned main/no-worker-gate contract failed, not merely compilation.
- Both mutations were restored with exact reverse patches before final tests.
- Allocation test runs with Ctrl+Alt+Right and Ctrl+Alt+C bindings, matching,
  trigger repeats/releases, plus plain-key paths; it measures pure logic only.
- The disable-path contract failed before adding cancellation of Windows'
  leftover local timer, then passed with the fix. The source-capacity/reuse
  regression also passes; that additional regression was not run failing-first.

## Verification scope

Clean environment means `env -i HOME="$HOME" PATH="$PATH" LANG=C.UTF-8`;
no DISPLAY/WAYLAND/XAUTHORITY/session bus variables are inherited. Cargo uses
`-j 2 --offline`, existing cached dependencies and worktree `target` on disk.
The sandbox initially denied loopback/private-socket workspace tests; rerunning
the same clean-environment command with socket permission passed. No real
desktop session was contacted. No `cargo fmt` was run.

Final results on 2026-10-31, implementation commit `c5b9143`:

| Check | Outcome |
| --- | --- |
| `cargo test -j 2 --workspace --offline` | PASS: 1,090 passed, 0 failed, 14 ignored, across 35 test/doc-test suites. Ignored tests remain unexecuted. |
| `cargo clippy -j 2 --workspace --all-targets --offline -- -D warnings` | PASS |
| Windows `cargo clippy -j 2 --target x86_64-pc-windows-msvc -p one-mk-input -p one-mk-proto -p one-mk-ctl -p one-mk-app -p one-mk-tray --all-targets --offline -- -D warnings` | PASS, compile check only |
| macOS, same package/target flags with `--target aarch64-apple-darwin` | PASS, compile check only |
| `git diff --check` | PASS |
| Native acceptance scripts, native idle CPU/RSS/latency and actual key delivery | NOT RUN; UNTESTED |

Native cross checks do not execute code. The two controlled behavioral mutations
failed their targeted tests and were restored before this final suite.

## Hostile self-review / remaining acceptance blockers

The old dead-worker gate is gone, not replaced by another health prerequisite
on core capture. Disabled Local is removed; Windows failed native cleanup is a
separate parked, release-only object. A delayed Mac local-output tag must remain
recognized after off or cleanup could be forwarded remotely. This narrow
cleanup state is intentional and does not admit/remap physical keys.

Reviewing the fixes found two additional issues and corrected them: suppressed
owned ups needed queue retirement even on failed health, and side-agnostic
aggregate flags could survive when only the opposite modifier side was stale.
The failure-up and opposite-side tests now cover these cases. A final bounded
source-table review also found silent capacity exhaustion; correlation now
explicitly fails local mapping and releases output instead of losing source
ownership, with a capacity/reuse regression and native caller contract. Power notifier
startup/teardown is scoped to opt-in rather than imposing it on feature-off
users. A disable-path source regression also reproduced a leftover Windows local
timer after dropping the interceptor; disable now explicitly cancels that timer.
Native start/quit paths were rechecked after cross-compilation.

**Not production-ready:** no native acceptance, no guaranteed OS release after
permission/UIPI loss or kill, best-effort local/remote boundary ordering, mixed
keyboard-type deferred provenance, device-correlation/passthrough reordering,
macOS Caps Lock unsupported, and no measured native idle CPU/RSS/latency. In
particular, this report does **not** certify "enterprise-level / no known bugs"
or fully resolve those delivery/provenance limitations. Default OFF and
EXPERIMENTAL must remain until the disposable-session scripts in
`../LOCAL-REMAP.md` pass separately on Windows and macOS.

## Owner-only feature gate and regression remediation

Branch: `t3/gate-native-remap-feature`. Reset baseline: `ad0f430` on local
`wave5`. Scope: latest `WAVE5-CODEX-REVIEW-NATIVE-REGRESSION.md`, findings R1–R6.
This section supersedes conflicting first-round claims, not the review itself.

### Release containment

`one-mk-input/native-local-remap` and its daemon forwarding feature have empty
defaults. Default Windows/macOS backends are separate source files, restored
from pre-5H `67cafc4`. The experimental copies, native worker/power modules,
receipt hotkey filters and native `LocalRules` commands are feature-selected.
Pure localmap/handoff/runtime modules remain always compiled. The app/tray
launch the daemon rather than linking native capture, so no UI-only feature
is necessary. Without the feature, `local on` prints exactly the missing-build
explanation; a disabled preference does not add an unsolicited status warning.

The default-source/main audit and independent 5I/typing exceptions are itemized
in `../LOCAL-REMAP.md`. `win_logic.rs` has no diff against main. New pinned-main
physical callback fixtures supplement existing activation fixtures. Contracts
run in both configurations; they assert module selection, absent native-local
symbols in default files, empty defaults and no feature-enabled CI/release
commands. These are source/pure differential contracts, not native execution.

### Finding-by-finding evidence

| Finding | Implementation / regression evidence | Status |
| --- | --- | --- |
| R1 HIGH: unobserved macOS ups destroyed the ledger | Both directions wait for exact tap receipt on worker. Up held/type/flag state is retired only after receipt; Engine also refuses to retire unconfirmed ups. Failing-first `unconfirmed_releases_retain_obligations_until_recovery` failed at retained key 29 before the fix. It now verifies failed Remote fencing and successful later retry. Existing empty-fence/lost-down tests remain. | Source/pure corrected; native UNTESTED |
| R2 HIGH: swallowed physical keys missed takeover | HID-only, controlled/debounced LocalInput notification precedes local admission/early return. Failing-first `swallowed_physical_takeover_precedes_local_admission` source harness failed its ordering check before relocation; it also pins the physical/inactive guard. Existing LocalGate pure tests cover debounce. | Wiring corrected; real takeover UNTESTED |
| R3 HIGH: remote keys consumed as local cleanup | Correlation envelopes carry optional local sequence identity; only the exact admitted local sequence may pop local waiting state. Missing/mismatched local state fails closed and consumes the local envelope, never a remote event. Orphaned local envelopes cannot fall through to remote transmission after detach. Failing-first `local_dispatch_requires_its_own_sequence_not_a_matching_remote_key` source harness failed before the guard; pure `mixed_local_and_remote_envelopes_keep_origin_even_for_identical_keys` exercises equal and different codes and queue flushing. | Source/pure corrected; native interleaving UNTESTED |
| R4 HIGH: release fence failed open | Installed experimental Local defers activation until matching successful completion; failure/400 ms expiry cancels the request. No capture callback waits on a worker. Failing-first `activation_waits_for_serialized_release_completion` source harness failed before deferred activation wiring; completion order is pinned and pure failed-release/empty-fence tests keep Remote closed until recovery. | Source/pure corrected; OS ordering UNTESTED |
| R5 MEDIUM: stale mapper tombstones swallowed fresh presses | Fence work carries the fresh physical baseline behind admitted work. Mapper reconciliation retires absent source keys, preserving still-held tombstones. Failing-first `missed_up_baseline_retires_only_absent_tombstones` failed the fresh-down assertion; it now also verifies a still-held repeat is not repressed. | Pure corrected; native snapshot accuracy UNTESTED |
| R6 MEDIUM: unconditional 5I timing regression | Correlator starts disabled; applicable per-device settings explicitly enable it. Local-only rules cannot enable it without both feature and preference. Disabled hooks forward immediately, device arrivals cannot arm deadlines, disabling flushes in order. macOS allocates a correlation timer only for pending work and invalidates it when empty. Failing-first `default_attribution_has_no_latency_or_deadline` failed because output length was zero rather than one. Disable/flush, settings-gate and unplug-order tests pass in both builds. | Default and experimental source/pure corrected; timing on hardware UNTESTED |

Failing-first logs were captured under ignored `target/` during implementation
(`r1-before.log`, `r5-before.log`, `r6-before.log`, `contracts-before.log`).
R2/R3/R4 native wiring assertions are intentionally described as source
harnesses, not a simulated OS or proof of native delivery. Regression test
sources and main fixtures are committed; transient logs are not release assets.

### Hostile self-review

The final pass checked all correlation output paths, including unplug/release,
disable and missing dispatch. It added orphan-local suppression and failure on
an empty local pending queue; a matching keycode alone is never sufficient.
Remote activation cannot use a failed completion; absent Local still follows
the original activation body. Default sources contain no local worker or receipt
path. No native hook/tap was installed or native code executed here.

Residual blockers remain: no native receipt/delivery acceptance, mixed
keyboard-type deferred provenance, approximate multi-keyboard correlation,
passthrough/mapped ordering, unsupported macOS Caps/Fn/media/function sources,
and no native idle CPU/RSS/latency measurements. Permissions/UIPI, a stuck OS
call, kill/abort/OOM or device loss can prevent release; a tap receipt is not
application delivery. Experimental crossing intentionally cancels on failed
cleanup. Separate experimental/default source files require parity maintenance;
the main contracts bound, but do not eliminate, drift. These risks are why the
feature remains absent from default/release/CI builds. Owner build commands,
disposable-session scripts, SSH/emergency recovery and exit criteria are in
`../LOCAL-REMAP.md`. No enterprise/no-known-bugs certification is made.

### Final validation

All commands ran with `env -i`, explicit HOME/PATH, isolated XDG configuration,
`CARGO_INCREMENTAL=0`, worktree-local Cargo artifacts and a private temporary
directory. No real-session hook/tap was installed. Cargo used `-j 2 --offline`;
no `cargo fmt` was run.

| Check | Default | Experimental feature |
| --- | --- | --- |
| `cargo test -j 2 --offline --workspace -- --test-threads=2` | PASS: 1110 passed, 0 failed, 14 ignored across 35 suites | PASS: same totals; `--features one-mk/native-local-remap` |
| `cargo clippy -j 2 --offline --workspace --all-targets -- -D warnings` | PASS | PASS; daemon forwarding feature enabled |
| Windows `x86_64-pc-windows-msvc` cross clippy | PASS, COMPILE-CHECKED ONLY | PASS, COMPILE-CHECKED ONLY |
| macOS `aarch64-apple-darwin` cross clippy | PASS, COMPILE-CHECKED ONLY | PASS, COMPILE-CHECKED ONLY |
| Final correlator and native source-contract tests | PASS | PASS |

Cross clippy used `-p one-mk-input -p one-mk-proto -p one-mk-ctl
-p one-mk-app -p one-mk-tray --all-targets -- -D warnings`, with
`--features one-mk-input/native-local-remap` only for the experimental runs.
The final default workspace lint also passed after the final assertion updates.
Ignored tests remain ignored, not represented as successful runtime coverage.

The inverse feature graph (`cargo tree --offline -p one-mk -e features
-i one-mk-input`) contains no `native-local-remap` in the default build and
shows daemon-to-input forwarding with `--features native-local-remap`.
`git diff main -- crates/input/src/win_logic.rs` is empty. Default backend
diffs against main were audited against the explicit independent-change table
in `LOCAL-REMAP.md`; activation and physical callbacks match pinned main fixtures
after only the documented device-attribution normalization. These bounded source
contracts do not prove whole-backend behavioral equivalence on native hardware.
`git diff --check` passed. Detailed command logs remain in ignored `target/`.
