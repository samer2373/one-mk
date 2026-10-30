# Wave 5 native regression review (Codex)

## Scope and conclusion

Reviewed `wave5` at `0dd87e8` (Merge native local remap fixes (5H-5) into wave5), after the requested reset. Compared native input changes with `main`, including shared input logic, device correlation, local-map workers, manifests, and native daemon call sites. Read the first review and remediation document, then traced the implementation independently.

**Not approved as exact-main parity or as completed native-remapping remediation.** Four HIGH and two MEDIUM findings remain. The original seven HIGH issues have substantial fixes, but restoring core capture liveness leaves release/transition safety gaps. Default-off operation still has an unconditional device-correlation timing difference. Permission-revocation behavior is another off-mode difference, classified below as intended-and-safe.

These are source-level findings, not observed Windows/macOS failures. Both native input target checks pass. Native delivery, permissions, scheduling and daemon target builds remain unverified on this Linux host. Scenarios involving lost native delivery explicitly state that precondition.

## Ranked findings

### R1 — HIGH — macOS counts unobserved releases as successful and destroys the retry ledger

**Classification:** regression, local rules enabled or cleanup after disable.

**Locations:** `crates/input/src/macos_localmap.rs:68`, `crates/input/src/macos_localmap.rs:77`, `crates/input/src/localmap_runtime.rs:374`, `crates/input/src/localmap_runtime.rs:385`.

The injector updates held keys, output modifier flags and its receipt-visible held flag before posting. It arms a receipt for every event, but the key-up branch at line 79 immediately sets `settled = true` and continues without observing that receipt. The successful prefix includes every posted up irrespective of delivery. Both normal emission and `Engine::fail` remove those ups from their ledger using that prefix.

**Scenario:** a mapped Control-down reached the OS. Cleanup posts Control-up while the tap is unavailable or delivery is lost during a permission/session transition. Unlike a down, the up is never checked. Cleanup reports success, the held indicator becomes empty, and the engine discards the release obligation. Recovery cannot retry it; Control can remain logically down. Source proves the false acknowledgement; whether a particular OS transition drops the post requires a Mac test.

**Suggested fix:** retain posted-but-unconfirmed releases separately. Do not classify posting as acknowledged delivery or empty the recovery ledger until the receipt arrives. Use asynchronous completion and bounded failure reporting, not capture-thread blocking. Test lost up receipts separately from lost down receipts.

### R2 — HIGH — macOS locally remapped typing no longer takes control back from a peer

**Classification:** regression, local rules enabled.

**Locations:** `crates/input/src/macos.rs:827`, `crates/input/src/macos.rs:837`, `crates/input/src/macos.rs:848`; contrast `crates/input/src/windows.rs:665`.

Physical keyboard events enter `local.hook` before the idle local-input takeover gate. A swallowed event returns at line 837. `Event::LocalInput` is only emitted later at lines 848–854. Tagged replacement events bypass this physical-input path. Windows carries an equivalent local-input flag through its pump; macOS does not.

**Scenario:** another computer controls this Mac (`controlled` set, local outgoing capture inactive). With a supported local map active, press an eligible physical key. Admission swallows it and reinjects its mapping, but the daemon never learns that real local input occurred. The peer continues injecting while the user types. Eligible identity/unmapped keys also traverse the worker and have this problem.

**Suggested fix:** report physical local input before the remapping early return, or defer a takeover notification through the pump. Preserve the HID-source, controlled-state and existing debounce checks. Add a controlled-peer plus swallowed-local-key test.

### R3 — HIGH — shared device correlation can consume remote keys as local cleanup

**Classification:** regression, local rules enabled; Windows needs device-specific local rules for the queued path, macOS does not.

**Locations:** `crates/input/src/windows_localmap.rs:252`, `crates/input/src/windows.rs:617`, `crates/input/src/windows.rs:728`, `crates/input/src/macos_localmap.rs:126`, `crates/input/src/macos_devices.rs:70`, `crates/input/src/macos_devices.rs:88`.

Local admission and remote forwarding share a correlator. Dispatch does not retain the event origin: every emitted key consumes `waiting.pop_front()` if a local key is waiting. A code/direction mismatch fails the local adapter but still returns `true`, discarding the unrelated key. Matching code/direction can instead assign the wrong event/device to the local key.

**Scenario:** hold locally mapped A, cross to a peer, and press remote B. B waits for Raw Input/IOHID attribution. Release the still-owned A before B flushes: its up is swallowed and admitted as cleanup. Admission draining queues A-up in the local FIFO and adds it behind B in the shared correlator. When B flushes first, `forward` pops A-up, fails the mapping and discards B. The eventual A-up can escape the now-empty local queue as a remote event. The transition fence flushes correlation at crossing, but cannot prevent this later overlap.

**Suggested fix:** carry origin and local sequence through correlation, or separate local and remote correlation queues with explicit ordering. Consume a local entry only for its own envelope. A mismatch must not silently consume an unrelated remote event. Test both different-key and same-key interleavings.

### R4 — HIGH — capture starts before the asynchronous local release fence completes

**Classification:** enabled-mode transition-safety regression. Immediate capture is an intentional liveness fix; exposing unresolved output is not safe.

**Locations:** `crates/input/src/windows.rs:808`, `crates/input/src/windows.rs:631`, `crates/input/src/windows_localmap.rs:279`, `crates/input/src/macos.rs:902`, `crates/input/src/macos.rs:482`, `crates/input/src/macos_localmap.rs:301`.

Both activation wrappers call `local_fence(Remote)` and immediately call `activate_now`/`start_now`. Admission closes and correlation flushes synchronously, but output release is merely enqueued to the worker. Neither completion nor failed/expired cleanup is consulted before emitting activation/modifiers and starting capture. The later watchdog can disable the adapter, but cannot undo the exposed transition.

**Scenario:** A emitted a synthetic modifier locally and the worker is delayed in injection/receipt handling. An edge crossing or `one-mk switch` activates the peer while local output remains held. Queued old local output/release can land after activation. If cleanup fails, remote control continues with unresolved local synthetic state. The ordering exists even without a timeout; a dead worker is the stronger case.

**Suggested fix:** preserve worker-independent capture for default-off/empty-ledger operation, but represent outstanding output as an explicit asynchronous retirement obligation. Quarantine further local output and establish release completion before declaring the remapped transition clean; expose bounded cleanup failure rather than treating enqueue as a fence. Do not restore the old unbounded capture/quit dependency. Test delayed and failed releases at edges and explicit switches.

### R5 — MEDIUM — physical recovery baseline does not reconcile preserved mapper tombstones

**Classification:** enabled-mode recovery regression.

**Locations:** `crates/input/src/localmap_runtime.rs:94`, `crates/input/src/localmap_runtime.rs:383`, `crates/input/src/localmap_runtime.rs:431`, `crates/input/src/windows_localmap.rs:235`, `crates/input/src/macos_localmap.rs:230`; tombstones at `crates/proto/src/remap.rs:779`.

`Physical::baseline` removes absent keys from hook ownership/source tracking but does not send the snapshot to the worker mapper. Failure calls `Mapper::release_all`, retaining `src.phys` and setting held records to `Rec::Swallow`. The repaired fence preserves this mapper via `set_keymaps`, retaining genuine held-key tombstones but also keys whose up was missed during an outage.

**Scenario:** mapped A-down reaches the worker; lock/Secure Input hides A-up. Failure creates an A tombstone. Recovery/reload sees A physically up and clears hook ownership, but not the mapper tombstone. The first new A-down is treated as the old held-key repeat and swallowed. Its up clears the tombstone, so the next press works. This concerns recovery of an existing adapter, not destruction/recreation through off.

**Suggested fix:** serialize physical-baseline reconciliation into the worker. Retire source records proven physically absent while retaining truly held tombstones. Separate baseline and across-fence tests do not cover this combined missed-up scenario.

### R6 — MEDIUM — device attribution changes ordinary OFF-mode latency and timer activity

**Classification:** regression against exact-main OFF behavior, although attribution is an intentional separate feature.

**Locations:** `crates/input/src/windows.rs:711`, `crates/input/src/windows.rs:733`, `crates/input/src/macos.rs:923`, `crates/input/src/macos_devices.rs:157`, `crates/input/src/devcorr.rs:24`, `crates/input/src/devcorr.rs:102`.

Captured input always enters the correlator, including absent, unsupported and disabled local rules. Keys/buttons/scroll tokens without already-available device data can wait `WAIT_MS = 8`; later events, including motion, queue behind them. Pending attribution arms Windows/CF timers. The first `MAX_MISSES = 8` misses incur waiting even when attribution never reports data, until matching data resets that count. Main forwarded immediately.

**Scenario:** a default-config user controls a peer and a hook/tap key precedes its device event, or attribution is unavailable. The remote key waits for attribution/timeout; a following pointer/scroll event can wait behind it. This is more than extra metadata. Eight milliseconds is the nominal window, not a guaranteed scheduling upper bound.

**Suggested fix:** demand-enable waiting correlation for an active device-dependent consumer or explicit learning session. Keep direct forwarding for ordinary no-device-rule operation. Test OFF, unsupported and after-off timing independently of adapter presence.

## Recheck of the first review's seven HIGH findings

| Original issue | Independent code disposition |
| --- | --- |
| 1. Every Windows power broadcast ends capture | **Original fixed.** `windows.rs:1203` restricts power handling to suspend/resume values 4/7/18; line 1208 returns no core effect without an adapter. Unrelated power-status broadcasts no longer end capture. Relevant events update awake/recovery state; enabled rules can still pause/end. |
| 2. Windows activation/hotkeys depend on worker health | **Core liveness fixed; R4 remains.** Activation is immediate (`windows.rs:808`). Ordinary shortcut decisions do not require worker recovery (`localmap_runtime.rs:135`); disabling removes interception. Existing hotkeys no longer require a worker fence. |
| 3. macOS crossing/quit depends on cleanup receipts | **Core liveness fixed; R1/R4 remain.** `macos.rs:902` starts immediately. Quit unconditionally stops its run loop in the command path (`macos.rs:785`). Ups no longer block on capture callbacks, but posting is incorrectly counted as successful release. |
| 4. Windows repeat stays suppressed after off/Remote gate | **Original fixed.** Empty/unsupported/off rules drop `Local`; release closes the local gate, retaining normal no-local repeat/held-key handling. A visit alone does not install a permanent suppressor. R6 affects timing, not repeat admission. |
| 5. Unknown Alt-up swallowed, leaving OS Alt held | **Original fixed.** Ownership tracks current holds; unknown-owner ups pass (`localmap_handoff.rs:183`). Baselines retire absent hook-owned keys instead of treating every configured key as permanently owned. R5 is a separate stale worker-state issue. |
| 6. Windows release retry missing on unlock/desktop change | **Retry plumbing fixed.** Unlock (`windows.rs:1335`) and desktop handling (`windows.rs:1175`) retry releases; detached recovery can retry after `Local` removal. A physically held baseline no longer rejects recovery wholesale. Actual UIPI/secure-desktop delivery remains untested. |
| 7. macOS stale physical modifier flags create phantoms | **Original fixed.** `macos_localmap_logic.rs:57` intersects swallowed flags with actual event flags, including side bits. `macos_localmap.rs:273` preserves original flags while inactive/paused. Off removes the adapter. R1 concerns synthetic release obligations, not that repaired physical intersection. |

## OFF-mode differences and verified-OK paths

Intended-and-safe means an identifiable safe purpose in source, not exact equality with main or native acceptance.

| Area / scenario | Classification and evidence |
| --- | --- |
| Capture activation/release, edge crossing and explicit switch without `Local` | **Core control-flow parity verified**, except attribution/metadata. The fence returns without an adapter; activation/release does not require worker admission. `win_logic.rs` is unchanged. |
| Repeat and held keys after ordinary visit/off | **Admission parity verified.** Disabled/unsupported/empty configuration leaves no live remap interceptor. Main's Windows held-at-crossing behavior remains. |
| Windows RegisterHotKey and hook shortcuts | **Existing-action OFF routing preserved.** Physical-hotkey interception resets on `Local` drop. New broadcast/fetch-clipboard actions are **intended-and-safe extensions**, not altered existing meanings. |
| Power/session handling OFF | **Original forced-release regression fixed.** `windows.rs:1208` returns no core effect without `Local`. Returning 1 instead of default processing for recognized suspend/resume messages is an **intended-and-safe acknowledgement difference**. New awake bookkeeping is inert to OFF capture. Existing session-lock handling remains. |
| macOS tap timeout OFF | **Original regression fixed.** Forced end depends on held local output; ordinary no-local timeout takes the main re-enable path. |
| Input Monitoring revoked with an existing Mac tap | **Intended-and-safe hardening difference.** `macos.rs:602` checks permission before accepting an existing tap; line 619 ends capture and reports/retries permission recovery. Main lacked that specific existing-tap check. Therefore this is not exact OFF parity. |
| Captured keyboard/button/wheel and queued motion | **R6 regression.** Attribution can delay all these paths. Underlying pointer scaling, button/scroll conversion and existing injection formulas are otherwise preserved. |
| Device metadata and native callbacks | **Intended-and-safe extension with resource differences.** Raw Input/IOHID add device identities even OFF. `macos_devices.rs:95` creates a process-lifetime manager/context and CF timer, parked far in the future without pending work. Not zero-resource parity; the bounded process-lifetime allocation is not an accumulating per-enable leak. |
| Healthy home idle CPU without rules | **No new recurring remap thread/poll found.** Workers/watchdogs/power subscription are enabled-adapter resources. Attribution adds transient captured-input timers (R6), not a steady home-idle polling loop. macOS retains its one-second permission/screen tick and adds the parked device timer. This is source analysis, not measured CPU. |
| Off during failed Windows cleanup | **Intended-and-safe recovery exception.** A release-only worker can outlive the interceptor to retry held outputs without remapping input or gating capture. An absolute claim of no extra thread immediately after off in every failure case would be incorrect. |
| Tagged macOS cleanup posts after off | **Intended-and-safe lifecycle difference.** A local-post bypass remains to avoid recapturing delayed self-generated events. It does not restore physical interception. |
| Character/text injection API additions | **Intended-and-safe extension at source level.** Native physical fallback aliases existing key injection; unsupported native text is reported rather than changing existing Key/Motion/Scroll operations. Native typing was not exercised. |
| Panic and binary profile | **Release-profile parity verified.** Root manifest diff adds only workspace default members. Release retains abort, fat LTO, one codegen unit, stripping and opt-level 3; the temporary global unwind change is gone. Added dependencies/default members are **intended build differences**. Binary byte/size parity and native linking are not certified. |

Traced shared `lib.rs`, native modifier/key conversion and hotkey backends, handoff/runtime, device correlation, and daemon native snapshot/reload call sites. Examined daemon imports, `CaptureCmd::LocalRules`, methods and calls are consistently cfg-guarded (`crates/one-mk/src/app/tune.rs:25`, `crates/one-mk/src/app.rs:802` and subsequent reload sites). No additional definite target-cfg defect was found. Input-crate cross checks do not compile native daemon bodies; static inspection is not a native daemon build, particularly given the stated `ring` cross-build limitation.

## Validation and limitations

Commands used clean `env -i`; no owner display/desktop/DBus variables were passed. Tests used isolated `XDG_CONFIG_HOME` under ignored checkout-local `target/`, and builds stayed on disk there rather than `/tmp`. No formatter or live desktop/clipboard/native backend was run. No source modifications were made.

- PASS: `cargo check -j 2 --offline -p one-mk-input --target x86_64-pc-windows-msvc`.
- PASS: `cargo check -j 2 --offline -p one-mk-input --target aarch64-apple-darwin`.
- PASS: `cargo test -j 2 --workspace --offline`: 1,099 passed, zero failed, 14 ignored across 35 successful result summaries, including zero-test/doc-test suites. All three native source-contract tests passed.
- Initial sandboxed workspace testing could not bind sockets; custom temporary-directory permissions also invalidated security fixtures. Those were environmental failures, not source findings. The complete clean-environment suite was then repeated once with approved socket access and ordinary small temporary fixtures, and passed. No owner configuration was accessed for application operation.

The source contract verifies its selected textual assertions, not native state-machine equivalence. It does not cover swallowed takeover, mixed-origin correlation, worker-baseline reconciliation or lost release receipts. Those require behavioral regression tests and Windows/macOS acceptance runs before remediation can be considered complete.
