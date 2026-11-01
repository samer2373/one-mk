# Local keyboard remapping: safety design (5H)

**2026-11-02 implementation candidate:** `native-local-remap` is now a default
Cargo feature in `one-mk-input` and `one-mk`, so ordinary release builds include
the native adapters. This is not a claim of release acceptance: **5H is still
incomplete, especially macOS Caps Lock. Windows/macOS runtime acceptance has
NOT RUN.** The owner requested release delivery; the remaining blockers below
must not be mistaken for completed work.
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
  five standard mouse buttons through the same ownership/release ledger.
- The physical Ctrl+Alt+Shift+Esc emergency closes admission immediately,
  releases worker output and tells the daemon to persist the preference off.
  A later focus or device notification cannot silently turn it back on.
  The emergency flag is independent of the bounded shortcut queue, including
  when that queue is full or another shortcut follows the emergency.
- macOS disable retires the worker without joining it on the capture loop.
  Release receipts still reach the retired ledger after interception stops;
  failed releases stay in release-only recovery and block a new local worker.
  This avoids preventing the tap from observing the worker's own release.
- Equal per-device maps are no longer registered as device overrides. Ordinary
  global/OS rules bypass device correlation even with keyboards connected.
  Actual per-device overrides retain the existing bounded correlation wait
  (up to 8 ms), so the requested 1 ms end-to-end budget is **not met for them**.
- The window is only a client of the daemon. Its exit/crash does not terminate
  the input worker or discard the ledger. Daemon abort/kill, OS injection
  refusal and unavailable permissions still cannot guarantee delivered ups.

### Caps Lock: unresolved release blocker

Caps Lock remains rejected on macOS, including as an output. The window and
native status say so. Merely swallowing a Caps Lock `FlagsChanged` event does
not provide physical down/up edges or correctly own the OS toggle/LED state.
No `hidutil` command is run by this candidate. `macos_hidmap` now generates the
documented UserKeyMapping property for pure 1:1 rules, including Caps-to-Esc,
Caps-to-Control and swaps. It rejects chords, layers, tap/hold, mouse buttons
and ambiguous HID usages rather than silently changing their meaning. Pure
generator tests do not implement the native mapping lifecycle or demonstrate
correct LED state; the generator is deliberately not connected to activation.

Apple's documented UserKeyMapping mechanism (Technical Note TN2450) is the
appropriate system path for simple mappings. Integrating it still needs
transactional preservation/restoration of existing user mappings, physical
source identity and release ownership across KVM crossings, live rule changes,
device removal and emergency disable. A tempting Caps-to-F20 alias is NOT used:
it collides with a real F20 key. Direct Caps-to-Esc without source attribution
also changes what the existing remote mapper sees. Neither is a safe shortcut.
Complex Caps chords/layers/app rules need the same source/LED solution first.

Reference: https://developer.apple.com/library/archive/technotes/tn2450/

The contracts below remain the safety design; historical default-OFF and
remote-only scope statements are superseded by this delivery-change section.

## 2026-11-02 validation and handoff

All Cargo commands used `-j 2` and the dedicated target directory
`/home/samer/.t3/cargo-target/localremap`. No formatter was run.

| Check | Result |
| --- | --- |
| `cargo check -p one-mk-input -p one-mk-app --target aarch64-apple-darwin -j 2` | PASS; compilation only, no Mac runtime |
| Same check for `x86_64-pc-windows-msvc` | PASS; compilation only, no Windows runtime |
| Input library tests filtered by `local` | PASS: 65 tests, including interruption ledgers, emergency queue overflow and mapping generation |
| Daemon/window tests filtered by `local` | PASS: 15 daemon and 2 window tests |
| `native_localmap_contract` | PASS: 9 source contracts; not native delivery tests |
| `cargo clippy --workspace --all-targets -j 2 -- -D warnings` | PASS; run once at final validation |
| `cargo test --workspace -j 2` | PASS; run once at final validation; ignored tests remain unexecuted |
| Native tap/hook, Caps LED/state, latency and idle CPU | NOT RUN |

Owner's Mac acceptance checklist (not results):

1. Caps Lock to Escape: currently blocked, so do not count this feature as
   release-complete. After system mapping integration, verify physical make/break,
   repeats, LED state and a real Escape key independently, locally and across KVM.
2. A non-Caps chord: hold its source, then edit or disable the rule; verify every
   emitted key and modifier is released exactly once.
3. An app-specific rule: move focus into/out of the chosen app while its source
   is held; verify the old output is released and the next press uses the new app.
4. Physical Ctrl+Alt+Shift+Esc while holding a mapped key/button: verify local
   rules stop, the pointer comes home, and the switch stays off after focus changes.
5. Sleep/wake and lock/unlock mid-press: verify no output stays held, and any
   paused status is actionable in the window before resuming.
6. Cross to a peer while holding a mapped source, return, and crash only the
   window: verify crossing still works and the daemon retains release ownership.

## Non-negotiable core KVM invariant

Without the Cargo feature, the Windows and macOS backends contain no local
worker, snapshot, receipt, fence, tombstone adapter or hook/tap integration.
`one-mk keymap local on` says plainly **this build does not include the
experimental native local remap**; storing a preference cannot enable code
that was not compiled. Physical behavior follows main except the independent
changes enumerated below. Pure localmap/handoff/runtime tests always compile.

The default files are `windows.rs`, `macos.rs`, `macos_devices.rs`.
`lib.rs` selects separate `*_experimental.rs` files only with the feature;
native worker/power modules and hotkey receipt/interception glue are similarly
gated. The app/tray launch the daemon and do not link the input backend; build
the daemon explicitly with the feature for owner tests, not a UI-only binary.

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
| `Backend.keyboard: None`, unsupported `Inject::Text`, `CharacterKey` physical fallback including repeat | Independent typing integration; native character composition is not claimed |
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
`Event::Devices` also resolve/reapply snapshots. Consequently mapping **can
resume automatically on a settings/device reload**, including re-enumeration
after wake. It is not an explicit-reload-only policy. Unlock/desktop return
retry cleanup; they do not themselves promise local mapping resumed. Ask
`one-mk status` again after asynchronous reload. `active (N)` means the local
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
