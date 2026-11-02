# Local keyboard remapping: safety design (5H)

**2026-11-02 implementation candidate:** `native-local-remap` is a default
Cargo feature in `one-mk-input` and `one-mk`, so ordinary release builds include
the native adapters. Simple macOS mappings now use native, per-keyboard
UserKeyMapping properties with journaled restoration, and local per-device
attribution never waits for a later device sample. **Windows/macOS runtime
acceptance has NOT RUN.** Implementation and portable tests are not evidence
of correct native delivery or LED behavior.
`[input] local_rules = false` remains the default, including upgrades.
Linux local remapping is unavailable. Native release delivery, no-stuck-keys,
latency, idle CPU and memory acceptance have **not** passed.

## Window and native delivery changes

- Keys & mouse has an **Also on this computer** switch. It persists the existing
  preference through the normal tuning command path; no terminal or config
  edit is needed. Key rules and button rules show an **Also here** annotation.
- With the switch on, global and matching-OS key/button rules apply locally as
  well as remotely. Existing explicit local scopes remain local-only and take
  priority. App-specific snapshots follow the local foreground application;
  remote resolution and its scope priorities are unchanged.
- The window shows native status and unsupported-source explanations in plain
  words. Linux clearly says **Not available on this desktop**. Neither X11 nor
  Wayland local interception is implemented in this candidate.
- The macOS intercepting tap is session-level. Tagged replacements still enter
  through HID posting and bypass remapping at the tap. Windows retains the
  low-level keyboard/mouse hooks and injected-input bypass. Both now route the
  ownership/release ledger only for mouse buttons that are rule sources. Ordinary
  unmapped clicks pass through untouched; reposted macOS clicks carry their
  original per-event click count, including double/triple clicks.
- The physical Ctrl+Alt+Shift+Esc emergency closes admission immediately,
  releases worker output and tells the daemon to persist the preference off.
  A later focus or device notification cannot silently turn it back on.
  The emergency flag is independent of the bounded shortcut queue, including
  when that queue is full or another shortcut follows the emergency.
- macOS disable retires the worker without joining it on the capture loop.
  Release receipts still reach the retired ledger after interception stops;
  failed releases stay in release-only recovery and block a new local worker.
  This avoids preventing the tap from observing the worker's own release.
- Equal per-device maps are not registered as device overrides. Local input
  consumes an already cached matching device sample immediately or uses the
  ordinary, non-device map immediately. Late samples cannot change an admitted
  press's source, and its up retains the down's attribution. Local events never
  enter the 8 ms waiting queue or arm its timer, even behind remote input.
  Remote KVM attribution retains its existing correlation behavior. This removes
  the intentional wait; it does not measure the complete native latency budget.
- Closing or crashing the window does not change the saved switch or the
  daemon's rules. Daemon stop restores native mappings and owned outputs without
  saving the preference off; the next start recovers first and reapplies the
  saved preference. macOS and Windows acknowledge releases before a normal stop
  finishes (with a bounded timeout and an explicit error if not confirmed).
  Daemon normal exit and
  SIGINT/SIGTERM/SIGHUP/SIGQUIT all run the same restore path. SIGKILL/abort,
  unavailable permissions and OS refusal cannot promise immediate delivered ups;
  the persistent journal allows recovery on the next daemon start.

### macOS Caps Lock and native mapping ownership

Pure unconditional 1:1 rules, including Caps-to-Escape, Caps-to-Control and
swaps, now use the IOKit equivalent of `hidutil property --set`: a
UserKeyMapping array on each matching keyboard service, using Apple's documented
simple-client constructor. No subprocess or
configuration edit is needed. macOS owns the actual Caps toggle/LED behavior;
oneMK does not swallow Caps transitions or substitute an F-key alias. LED/state
and real make/break delivery still need the owner's Mac acceptance run.

The native plan and tap plan are disjoint. Chords, layers, tap/hold, buttons,
app-specific sources and ambiguous keyboard usages stay in the tap plan. A
fixed-point collision check across all device profiles prevents a native output
from accidentally becoming a second tap remap. Conditional Caps rules are still
refused, not silently reinterpreted. Per-device mouse maps remain in the tap.

The lifecycle reads each service's existing mappings, retains foreign entries,
refuses conflicting sources and does not claim identical preexisting entries.
Before changing any service it atomically writes and syncs
`local-key-mapping.json` in the settings folder, recording only the entries it
will add and that service's registry ID. A second read rejects observed
concurrent changes; native readback verifies each write. Restoration re-reads
the property and removes only recorded exact pairs, retaining foreign edits.
Disconnected services are forgotten. Malformed property/recovery data is not
overwritten, and failed restoration retains the recovery record. IOKit has no
compare-and-swap for this property: an external tool must not race the final
property write with an indistinguishable replacement of oneMK's exact entry.

Startup recovery finishes before any new plan is applied. Its failure disables
local rules with a window-visible reason, not input capture or edge crossing.
IOKit registry IDs are borrowed CFNumbers, read as signed 64-bit values and
preserved as u64; a Get result is never released by oneMK. Disable, real rule
changes, emergency off, sleep/lock, tap timeout/user-disable and daemon shutdown
restore the native plan. A dedicated worker owns IOKit and all journal I/O,
including fsync; the tap/run loop only queues work and consumes acknowledgments.
Services are retained in a cache for property reads/writes, refreshed once per
nonempty apply rather than rescanned for every operation. Equal desired plans
are skipped, empty plans do not enumerate keyboards, and no empty recovery
record is rewritten before deletion. Real changes still require durable journal
writes before native changes, and remote activation waits for restoration. Changed held sources or
outputs receive tagged release events before property changes. Physical HID
source state repairs the crossing modifier snapshot when native maps are removed
before forwarding; only oneMK-owned mappings participate in that repair. The raw
physical emergency chord also works when its keys have native mappings.

Fake-runner tests cover apply/replacement/restore, crash before/after a native
write, recovery failure/retry, concurrent foreign edits, identical foreign
entries, source conflicts, partial rollback, per-device scope and disconnect.
These tests do not simulate IOKit, the event server, native LED state or delivery
of the extra tagged releases. Native held-key/crossing acceptance is mandatory.

Reference: https://developer.apple.com/library/archive/technotes/tn2450/_index.html

The contracts below remain the safety design; historical default-OFF and
remote-only scope statements are superseded by this delivery-change section.

## Review fixes: owner hardware checks — NOT RUN

This follow-up is implemented on Linux. Portable tests execute here; native
macOS/Windows backends and their text-capability tests are cross-compiled, not
run. CI now denies warnings for all-target cross-checks of input, ctl, proto and
app on both native targets. The daemon itself is not cross-compiled on this box.
No native delivery, latency, permission or zero-idle-CPU result is implied.

Validation on this Linux host (default features, dedicated target directory):

| Check | Result |
| --- | --- |
| `cargo clippy -j 4 --workspace --all-targets` | PASS, no warnings |
| `RUST_TEST_THREADS=4 cargo test -j 4 --workspace` | PASS: 1,361 passed, 0 failed, 18 ignored |
| input/ctl/proto/app, `--all-targets --target aarch64-apple-darwin`, `RUSTFLAGS="-D warnings"` | PASS, including text-capability test compilation |
| input/ctl/proto/app, `--all-targets --target x86_64-pc-windows-msvc`, `RUSTFLAGS="-D warnings"` | PASS, including text-capability test compilation |

An initial unrestricted-concurrency workspace run timed out in
`portal::tests::keysym_pairing_repeats_aliases_and_visit_cleanup`; it passed
alone and in the complete four-thread retry. Both previously known flaky
tests also passed in the complete retry. No portal implementation was changed.

- [ ] Hold remapped Ctrl/Alt across unrelated focus changes and Alt+Tab. Outputs
  must stay held. Change an actually applicable rule while held: the old rule
  lasts until all keys/buttons are released, then the new snapshot takes over.
- [ ] Enter/leave Secure Input, sleep, lock, tap-disable or UAC/secure desktop.
  Release every key; local rules must resume without toggling the preference.
  The emergency chord is different: it persists off and must NOT auto-resume.
- [ ] With a mouse rule on only button4, ordinary left/right/middle clicks must
  remain native. Test double/triple clicks on an explicitly mapped macOS button;
  on Windows verify unmapped clicks in elevated apps and protected games.
- [ ] Corrupt/version-bump the recovery journal in a disposable Mac account.
  Expect a clear local-rules-unavailable status, unchanged foreign properties
  and working pointer sharing. Restore the journal and restart for recovery.
- [ ] Trace rapid crossings with several keyboards and a slow settings disk.
  Confirm IOKit/journal work is off the tap thread, ordering is journal-before-
  install and restore-before-forward, and no modifier gets stranded.
- [ ] Close/reopen the window, relaunch/update, stop/restart the daemon and log
  out/in with local rules enabled. Closing alone must not disable; daemon stop
  restores original mappings, keeps the preference, and next start reapplies it.
- [ ] Crash/restart on the same keyboard registry IDs; verify the journal finds
  exactly those keyboards, preserves foreign mappings and removes owned entries.

## Earlier 2026-11-02 validation and handoff (before review fixes)

All Cargo commands used `-j 2` and the dedicated target directory
`/home/samer/.t3/cargo-target/localremap`. No formatter was run.

| Check | Result |
| --- | --- |
| `cargo check -p one-mk-input -p one-mk-app --target aarch64-apple-darwin -j 2` | PASS; compilation only, no Mac runtime |
| Same check for `x86_64-pc-windows-msvc` | PASS; compilation only, no Windows runtime |
| Input library tests filtered by `local` | PASS: 79 tests, including lifecycle/crash recovery, foreign ownership, emergency and no-wait attribution |
| Daemon tests filtered by `local_native` | PASS: 5 tests for Caps/swaps, app focus, applicable scopes, mouse/device scopes and cross-profile collision fixed point |
| `native_localmap_contract` | PASS: 11 source contracts; not native delivery tests |
| `cargo clippy --workspace --all-targets -j 2 -- -D warnings` | PASS on the Linux host; run once for this follow-up |
| `cargo test --workspace -j 2` | PASS: 1,323 passed, 0 failed, 18 ignored; run once for this follow-up; ignored tests remain unexecuted |
| Native tap/hook, Caps LED/state, latency and idle CPU | NOT RUN |

Owner's Mac acceptance checklist (not results):

1. Start with keys released and **Also on this computer** off. Grant the existing
   Accessibility/Input Monitoring permissions. Add Caps Lock → Escape in Keys &
   mouse and turn the switch on. Verify real Escape, repeated Caps presses,
   physical make/break and Caps LED/toggle state; disable mid-press and verify no
   Escape stays down and the prior Caps behavior returns.
2. Repeat with Caps → Control and a two-key swap (including Caps ↔ Escape).
   Check the modifier release, another real Control key and both keyboards when
   only one keyboard is selected. Existing mappings from another keyboard tool
   must remain unchanged before, during and after activation.
3. Add a non-Caps chord, then an app-specific rule. Hold each source while
   editing/disabling its rule or changing foreground application. An unchanged
   effective rule must keep its output held. A real swap waits until all keys
   are released; the next press follows the new rule/application. Conditional
   Caps rules must explain that they are unavailable rather than partially run.
4. Hold mapped keyboard/mouse output and press physical Ctrl+Alt+Shift+Escape.
   Verify releases, pointer back here, switch off, and no automatic re-enable on
   focus/device changes. Repeat with Escape or a modifier itself remapped.
5. Sleep/wake and lock/unlock mid-press for Caps → Control and a tap chord.
   Verify no stuck modifier/output, native restoration, clear paused status,
   and automatic resumption after the session is available and every key is up.
   Also exercise macOS tap timeout/user-disable in a disposable test session.
6. Cross to a peer with Caps or another mapped source held; check raw source
   identity/modifiers remotely, return, and release. Repeat rapidly on both
   keyboards, with a mouse-button device rule and with unknown attribution. There
   must be no 8 ms local delay; measure the requested ≤1 ms budget separately.
7. With mappings enabled, quit the window; confirm rules and saved preference
   stay on. Stop the daemon normally and with each handled signal: the recovery
   record must clear, original mappings return, and the saved preference stay on.
   Restart and verify rules reapply. Kill only the window mid-press: the live daemon must retain release
   ownership and the emergency chord must still work.
8. In a disposable session, abruptly kill the daemon after applying mappings.
   Restart it with the preference off and verify recovery occurs before any new
   rules: only oneMK's entries disappear, foreign mappings survive and the
   recovery record clears. Repeat with a disconnected/reconnected keyboard.
9. Measure idle CPU and local event latency on both native OSes. Cross-compiling,
   deterministic no-wait tests and source contracts are not runtime measurements.

## Non-negotiable core KVM invariant

Without the Cargo feature, the Windows and macOS backends contain no local
worker, snapshot, receipt, fence, tombstone adapter or hook/tap integration.
`one-mk keymap local on` says plainly **this build does not include the
experimental native local remap**; storing a preference cannot enable code
that was not compiled. Physical behavior follows main except the independent
changes enumerated below. Pure localmap/handoff/runtime tests always compile.

The feature-off files are `windows.rs`, `macos.rs`, `macos_devices.rs`.
`lib.rs` selects separate `*_experimental.rs` files with the default feature;
native worker/power modules and hotkey receipt/interception glue are similarly
gated. The app/tray launch the daemon and do not link the input backend; build
the daemon as well as the window for owner tests. Ordinary daemon release builds
include the feature; a UI-only binary cannot update an older daemon.

Disabled/empty/unsupported snapshots remove the local interceptor, rather than
leaving an unhealthy `Local` instance on the capture path. No local flag rewrite
or admission decision runs without an interceptor. Off/empty status clears the
local-problem line; doctor does not warn about native local rules unless a
local status line actually exists. Unsupported configured snapshots report why
they were refused. Existing permission diagnostics are independent.

Cleanup of output **already injected before disable** is the necessary exception:
it may post replacement key-ups, never new downs, and must not intercept new
physical input. Windows retains a release-only recovery worker if native ups
fail, wakes it on foreground/desktop/unlock events, and refuses only a *new local
mapping enable* until that old ledger is empty. It does not gate capture. macOS
recognizes its previously issued local-output tags after disable so delayed
cleanup cannot become remote input. These tags, like the normal injector tag,
are routing markers, not authentication against another local process.

`tests/native_localmap_contract.rs` pins capture activation and physical event
callbacks from `main` (`96436057bd1a303159e07953d29c2d254eb7314d`), allowing only
the documented per-device metadata additions. It checks default source isolation
and experimental completion wiring in both feature builds.
Random pure tests compare closed-home routing/repeats/unowned ups
and disabled modifier flags against main's pass-through behavior. These are
**bounded differential/source contracts, not an OS execution proof**: the full
native event-loop behavior still needs the manual acceptance below.

### Differential audit against main (feature absent)

`git diff main -- crates/input/src/windows.rs crates/input/src/macos.rs
crates/input/src/win_logic.rs` was inspected hunk by hunk. The first two were
restored to the pre-5H implementation (`67cafc4`), retaining only these
independent differences; `win_logic.rs` is byte-identical to main:

| Difference | Reason |
| --- | --- |
| Windows Raw Input imports, message/timer IDs, `State.raw`, startup enumeration, registration on activation, unregister/flush on release, device-change/input/timer dispatch, device-aware send path | 5I device discovery/attribution, not local remapping |
| macOS `Ctx.hid`, IOHID startup, capture enable/disable on start/end/release/quit, correlated remote send path | 5I equivalent; queued events flush before unplug notification |
| `device: None` on captured keys/buttons/motion/scroll | 5I event-schema metadata; disabled correlator forwards immediately without a deadline |
| `CaptureCmd::DeviceRules` and correlator opt-in; lazy macOS timer allocation | R6 removes unconditional 5I delay/timer; only per-device rules opt in while the backend captures input |
| Shared `macos_inject.rs` / `windows_inject.rs`, keyboard capability construction and CharacterKey/Text repeat/cleanup | The review follow-up shares the complete typing injector with both feature configurations; the original physical-only exception was a defect |
| `macos::ax_trusted` visibility only | Existing permission query in `lib.rs`; no permission behavior change |

There are no remaining 5H native-local hooks in these default files. The
5I/typing exceptions mean this is **not a byte-identical whole backend** or a
native execution equivalence proof. Both configurations run the same pinned-main
differential contracts. CI/release commands enable neither this feature nor
workspace-wide `--all-features`.

## Ownership and worker contract

The capture hook performs physical shortcut recognition and bounded admission,
not mapping or injection. The worker owns `Mapper`, deadlines, output ledger
and ordered fences. Admission is bounded to 64 unacknowledged events with a
100 ms oldest-work lease. Snapshot retirement and device correlation happen
off-hook. Worker/lease threads park indefinitely without obligations; there is
no periodic idle timer. Held keys, outstanding admissions and undecided dual
roles are obligations, not idle. Actual native CPU/RSS has not been measured.

Closing admission happens before a fence or failure. Ordinary shortcuts keep
health intact and own only their swallowed trigger; their release never enters
the mapper. Physical Ctrl+Alt+Shift+Esc is the emergency: fail admission, release
replacements and ask to return home, while allowing emergency Esc through. A
previously rewritten modifier is not retroactively delivered to the OS; do not
claim that the OS receives the full physical emergency chord.

**Inside the experimental build only**, an installed local interceptor closes
admission and serializes a Remote fence before activation. Capture starts only
after the matching successful completion, never while releases are outstanding.
The callback does not block: a failed/expired fence cancels activation and
reports local failure (400 ms watchdog); recover/disable local mapping before
retrying. This deliberately sacrifices experimental crossing availability rather
than overlapping local replacement downs with remote capture. Without an
interceptor, activation immediately follows main. Without the Cargo feature,
no experimental fence can gate the core KVM at all.
Returning home closes the gate in every case, then requests Local only if
mapping remains enabled and unpaused. Thus paused/off Backspace repeat stays
with the OS instead of being suppressed by a stale Remote gate.

Fences release outputs and update keymaps on the **existing mapper**, preserving
release-all tombstones only for physically held keys. Recovery snapshots retire
absent source keys after the admitted queue drains, before reopening admission;
a missed up cannot swallow the next fresh press. A held key must come up before being interpreted under
a new map: no repeat-induced re-press or spurious dual-role tap. Owned ups while
closed/failed/remote retire source bookkeeping without injecting new downs.
Only a failed, drained engine may resynchronize a sequence gap to its fence
watermark; a healthy fence cannot skip undrained work. Recovery generation
checks reject stale completions. Local readiness and Remote capture require
their respective successful fence completions in the experimental build.

| Original down owner | Repeats after crossing | Original key-up |
| --- | --- | --- |
| OS / failed-open | Suppress only during actual Remote capture | Pass locally; never forward to peer |
| Local mapper | Release replacement; preserve swallow tombstone | Swallow and retire outstanding source |
| Physical shortcut | Swallow trigger repeats | Swallow trigger up; no mapper up |
| Remote peer | Receiver handles repeat | Remote capture's normal ownership rules |
| No observed down | Do not invent ownership from historical mapping | Pass locally |

Physical source tracking is bounded and distinguishes outstanding correlated
downs by device/code; repeats do not add ownership. Exhausting the bounded
source table fails local mapping and releases output instead of losing ownership
silently. A key ever having been
mapped is **not** ownership. Baselines clear absent owners, treat held keyboard
keys as OS-owned and exclude mouse buttons. Enabling from an Enter-down CLI
command is not refused. Local rules never enter the peer's mapper; global/OS/
peer rules retain their remote-only meaning. `--local --app` remains refused.

Release builds use `panic = "abort"` for the whole application as before 5H.
The native worker paths use explicit startup/channel/injection failures, not
`catch_unwind`, `unwrap`, `expect` or deliberate panic as recovery. This is
fail-stop, **not** an in-process panic-release guarantee. OOM, abort, forced
termination, a stuck native OS call or permission loss can prevent cleanup.
No in-process implementation can promise key-up after kill between down/up.
A separate recovery mechanism and native fault testing are prerequisites for a
production no-stuck-keys claim.

## Windows adapter — COMPILE-CHECKED ONLY / UNTESTED

The low-level hook precedes local/remote routing with physical shortcuts.
While a Local interceptor exists, the RegisterHotKey notification path is
ignored so replacement output cannot trigger a KVM shortcut. Without Local,
the original RegisterHotKey path is unchanged. Other injected input and
VK_PROCESSKEY bypass local mapping, retaining main behavior when local is off.

Only suspend (4) and resume (7/18) power broadcasts can pause local remapping;
charger/battery notifications never end capture. Without Local these added
power handlers have no effect. WTS lock handling retains main's remote-release
behavior. Foreground elevation and desktop changes pause mapping; unlock,
desktop return and a normal foreground retry failed native releases and refresh
physical ownership. UIPI can already be active before a foreground notification:
release-before-elevation is **not guaranteed**. Recovery-only cleanup survives
`local off` without retaining the interceptor. Recovery is event-driven, not a
retry poll. It cannot recover while the process or OS injector is unavailable.

SendInput's exact successful prefix updates the output ledger. Partial/failed
ups stay owned for retry. Scancode outputs/layers/sequences/none/tap-hold are
supported; actions and unsupported virtual/media outputs refuse the snapshot.
AltGr fake Ctrl, Caps Lock toggle/LED, reserved combinations, UAC and multi-
keyboard correlation remain native acceptance cases.

Snapshots without device rules bypass the correlator delay. While numbered
admissions await correlation/acknowledgement, an unnamed high-resolution
waitable timer wakes the capture pump at approximately 1 ms. It is canceled at
idle; no process-wide timeBeginPeriod is enabled for local mapping. Startup
refuses local mapping if that timer cannot be created. Device correlation may
still wait up to 8 ms. Passthrough mouse/ineligible keys can overtake queued
mapped keys: latency/order acceptance remains incomplete; a 1 ms wake request
is not a hard latency guarantee.

## macOS adapter — COMPILE-CHECKED ONLY / UNTESTED

The session event tap and IOHID correlator feed the shared worker. Supported
physical keys are posted using a private CGEventSource and tagged sequence.
Downs and ups wait at most 100 ms on the worker for their exact outstanding
tap receipt. An uncertain down triggers failure/release; an uncertain up retains
its held-key, keyboard-type and modifier release obligations for a later fence.
Capture never blocks on the receipt. CGEventPost has no delivery result; even
an observed tap receipt is **not proof an application consumed the event**.
Missing receipts keep experimental Remote activation closed; they do not become
false release acknowledgements. Native no-stuck-keys is not established.

Ordinary reposts carry KeyboardEventKeyboardType from admission; a held output's
up retains its down's type. Mixed keyboard types interrupting an undecided
multi-event tap/hold batch still need provenance work/native validation. ISO
section/backtick behavior is **not accepted** by compilation alone.

Modifier rewriting intersects tracked passed bits with the original event,
including side-specific device bits; a missed up cannot add a phantom Cmd or
Shift. Disabled/paused mapping returns original flags exactly. Re-enable takes
a fresh HID key baseline. Tap timeout/user-disable pauses/release-all when local
mapping exists, but ends remote capture only if local output was held. With no
Local, it only re-enables the tap as main did. Quit stops the capture run loop
without requiring any fence completion; cleanup detaches IOHID dispatch before
joining the worker. Each receipt wait is bounded, but a batch can require
multiple serial waits; an arbitrarily blocked OS call can still prevent shutdown.

Power notifications use IORegisterForSystemPower on the **capture run loop**,
not NSWorkspace delivery on the tokio main thread. Registration exists only
while opted in; failure refuses local mapping. Sleep/wake pauses mapping and
allows the power transition without waiting for application delivery. Lock
notifications and Secure Input/session checks before injection and at 100 ms
while keys are held supply additional interlocks. This is not a claim of
instant idle fast-user-switch notification. Failed/paused physical state is
cleared so a missed up does not leave a permanent 10 Hz poll.

Caps Lock, Fn, media and function-key sources/outputs refuse the whole local
snapshot; original toggle/media handling remains untouched. Accessibility and
Input Monitoring are independently required. Permission revocation can also
prevent releases; restoring permissions and toggling the preference is not a
guaranteed recovery of an up already lost by the OS.

## Reload and status semantics

`one-mk keymap local on|off` changes settings; ordinary settings reloads and
`Event::Devices` also resolve snapshots. Unchanged effective snapshots are
not resent, including on focus changes. Real snapshot changes wait until all
physical keys/buttons and owned outputs are released. Transient interruptions
such as Secure Input, sleep, lock, tap disable or desktop switches retry
cleanup and resume automatically once the condition clears and no keys are
held; no preference toggle or settings reload is required. Ask
`one-mk status` again after asynchronous recovery. `active (N)` means the local
worker fence completed, never that native acceptance passed. Both active and
inactive status use plain window-facing words; preference off and zero rules
are silent. Doctor never infers activation from preference alone. The emergency
shortcut additionally persists the preference off before future reloads.

## Disposable-session acceptance scripts — NOT RUN

Never install a hook/tap in an owner's real login for this checklist. Use a
disposable login, record original settings and arrange recovery from another
computer. Run each OS separately; record actual OS key state, not just logs.

Build on the actual test OS, with the owner's explicit approval:

```sh
# Legacy baseline: explicitly exclude native local adapters.
cargo build -j 2 --release -p one-mk --no-default-features
# Candidate: includes native adapters without an extra feature flag.
cargo build -j 2 --release -p one-mk
```

Keep the baseline binary separately before the second build overwrites it.
Use an isolated settings directory/account and install no autostart entry.
For Windows PowerShell use the same Cargo commands without the shell comments.
The feature alone does not opt in: add a supported local rule and run the
feature-built daemon plus its `one-mk keymap local on` command in that account.
Do not test native hooks/taps on this Linux validation host or in a real login.

### Windows

1. With local off and with no rules/unsupported rules, exercise edge/switch/
   broadcast/all six shortcut actions, external injection, IME and Backspace
   auto-repeat before/after remote visits. Repeat after on -> off. Unplug/plug
   the charger and trigger battery notifications while controlling a peer.
2. Enable a supported rule, e.g. `one-mk keymap add capslock esc --local` then
   `one-mk keymap local on`. Hold Enter during enable and hold a mouse button;
   release them and verify later remaps/repeats work. Verify status/check/doctor.
3. Test sided modifiers, AltGr, Caps Lock LEDs, sequences/none/layers/tap-hold,
   every physical shortcut, and injected replacements that resemble shortcuts.
   Ordinary shortcuts must not pause mapping; emergency must release/home.
4. Hold remapped/passed/dual-role keys across edge/switch/broadcast/hotkey,
   return, reload and device-table changes. No extra down/tap is permitted.
   Repeat with two keyboards holding the same code and unknown correlations.
5. Hold Caps -> Ctrl while requesting UAC with Ctrl+Shift+Enter; release during
   secure desktop, return/unlock and verify automatic release retry. Disable
   while UAC is active, return, and verify release-only cleanup still retries.
   Test Alt+Tab from an elevated window where the hook missed the down; the
   unowned Alt/Enter up must pass. Repeat foreground elevation without UAC.
6. Exercise lock/unlock, sleep/wake, worker exit, denied/partial SendInput,
   queue/sequence failure, unplug/replug and daemon loss while keys are held.
   Experimental capture must not activate before cleanup succeeds; failure
   cancels crossing. Disable/recover, then verify core crossing works again.
7. Measure on/off RSS, callback/ordering latency and truly idle wakeups/CPU.
   Confirm no armed local/correlation timer remains with no obligations.

### macOS

1. With local off/none/unsupported and after on -> off, verify core KVM,
   repeat, every hotkey, synthetic input and tap disable behave like main.
   Verify off status has no local problem/WARN; inspect permissions separately.
2. In this disposable login grant Accessibility/Input Monitoring separately.
   Add `one-mk keymap add a b --local`; enable while Enter is held. Test sided
   Shift -> Ctrl, typing/clicking, both release orders, unknown/device rules,
   two keyboards, reload mid-hold, layer/none/sequence and supported tap/hold.
3. Hold replacements across all capture/return paths. Delay/drop down/up
   receipts and stop the worker: crossing must wait/cancel, never fail open;
   Quit must stop. Verify
   actual OS release; record any post that failed to reach the destination.
4. Test tap timeout/user-disable, Secure Keyboard Entry (Cmd+Tab while a
   modifier is held), lock/unlock, fast-user-switch, sleep/wake, permission
   revocation and device re-enumeration. No phantom Cmd/Shift may survive.
   Check paused idle wakeups after a deliberately missed physical up.
5. Verify Caps/Fn/media/F-key rules refuse the snapshot without changing core
   capture. Verify original LEDs/toggle/media behavior remains untouched.
   Test ANSI/ISO/JIS and mixed keyboard types, especially section/backtick and
   an interrupted dual-role key. This is a known provenance acceptance gap.
6. Measure CPU/RSS/callback latency and shutdown time with missing receipts.
   Repeat on supported OS versions and with another injector and IMEs.

### Recovery on the affected host

Use physical **Ctrl+Alt+Shift+Esc** first. From another computer's SSH/session
**into the affected host/account**, run `one-mk keymap local off`; running it on
the other computer changes the wrong settings. Release physical modifiers and
return to the normal/unlocked desktop. Windows retries its retained cleanup
ledger there. Check actual OS state before re-enabling. Disable autostart and
restore recorded settings if the daemon is dead; sign out/reboot the disposable
login if keys/flags remain stuck. Do not keep using a faulty native mapping.

On macOS permission revocation or force-quit may stop interception but also
prevent cleanup. Restore permissions only in the disposable session; verify OS
flags explicitly. The emergency/SSH/revoke procedures are escape routes, **not
proof that already injected downs received ups**. Do not claim native no-stuck-
keys or ZERO idle CPU acceptance until these scripts actually pass.

See `reviews/WAVE5-NATIVE-REMEDIATION.md` for the finding-by-finding ledger,
mutation evidence, consciously incomplete items and verification scope.

## Exit criteria before release acceptance

Default-feature inclusion was changed at the owner's explicit request, but
release acceptance still requires separate Windows/macOS hardware results and
an independent hostile review. Resolve the Caps Lock system-mapping blocker
before calling this implementation complete. Record OS/device/layout versions, exact commit/features,
baseline comparison, callback latency distributions, idle wakeups/CPU/RSS and
actual down/up traces for every script above. Exercise lost receipts, partial
injection, delayed/dead workers, mixed local/remote identical keys, takeover,
missed physical ups, unplug ordering and stale fence completions. Every emitted
down must have a verified up or a documented out-of-process recovery outcome;
no stuck keys, phantom modifiers, first-press loss or premature activation may
remain. Resolve mixed keyboard-type deferred provenance and macOS source
restrictions or explicitly narrow the accepted scope. Compile checks and pure
tests alone never satisfy these criteria; no bug-free certification is claimed.
