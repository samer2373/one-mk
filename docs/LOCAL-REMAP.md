# Local keyboard remapping: safety design (5H)

**EXPERIMENTAL; OFF by default. Windows and macOS are COMPILE-CHECKED ONLY,
UNTESTED on native hardware. 5H is incomplete, not production accepted.**
`[input] local_rules = false` remains the default, including upgrades.
Linux local remapping is unavailable. Native release delivery, no-stuck-keys,
latency, idle CPU and memory acceptance have **not** passed.

## Non-negotiable core KVM invariant

With local rules off, none configured, unsupported, or after
`one-mk keymap local off`, the Windows and macOS backends must behave exactly
like `main` for physical input. An experimental worker must **never gate
remote capture, crossing, switching, hotkeys, power handling or key repeat**.
Failing local remapping is not permission to break the KVM.

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

`tests/native_localmap_contract.rs` pins the capture activation bodies from
`main` (`96436057bd1a303159e07953d29c2d254eb7314d`), allowing only the earlier
per-device Raw Input/IOHID metadata additions. It checks that capture wrappers
have no worker-health precondition and that disable/quit/timeout wiring remains
fail-open. Random pure tests compare closed-home routing/repeats/unowned ups
and disabled modifier flags against main's pass-through behavior. These are
**bounded differential/source contracts, not an OS execution proof**: the full
native event-loop behavior still needs the manual acceptance below.

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

Capture submits a **best-effort** Remote fence and immediately uses main's
capture activation path: zero worker wait, including with a dead worker or
missing receipt. It never waits for local completion to publish `Activated`.
This favors core KVM availability; it does **not** establish that native local
ups have arrived before remote activation. Failed releases are reported/retried
where possible; native ordering across the boundary is an acceptance blocker.
Returning home closes the gate in every case, then requests Local only if
mapping remains enabled and unpaused. Thus paused/off Backspace repeat stays
with the OS instead of being suppressed by a stale Remote gate.

Fences release outputs and update keymaps on the **existing mapper**, preserving
release-all tombstones. A held key must come up before being interpreted under
a new map: no repeat-induced re-press or spurious dual-role tap. Owned ups while
closed/failed/remote retire source bookkeeping without injecting new downs.
Only a failed, drained engine may resynchronize a sequence gap to its fence
watermark; a healthy fence cannot skip undrained work. Recovery generation
checks reject stale completions. Acknowledging local mapping readiness is
separate from authorizing core capture.

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

The existing HID event tap and IOHID correlator feed the shared worker. Supported
physical keys are posted using a private CGEventSource and tagged sequence.
Downs wait at most 100 ms for their exact outstanding tap receipt; an uncertain
down triggers failure/release. Ups are **best-effort posts without receipt
wait**. The pure mapper releases its bookkeeping, so missing up receipts cannot
wedge capture or shutdown. CGEventPost has no successful-delivery return:
posting an up is **not proof that an application or OS consumed it**, and there
is no guaranteed retry for an undelivered macOS up. Do not describe this as
acknowledged native release delivery or established no-stuck-keys.

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
joining the bounded worker. An arbitrarily blocked OS call is still outside
this shutdown bound.

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
inactive feature status retain EXPERIMENTAL/UNTESTED labels; preference off and
zero rules are silent. Doctor never infers activation from preference alone.

## Disposable-session acceptance scripts — NOT RUN

Never install a hook/tap in an owner's real login for this checklist. Use a
disposable login, record original settings and arrange recovery from another
computer. Run each OS separately; record actual OS key state, not just logs.

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
   Core crossing/switch must remain available; report local delivery failures.
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
   receipts and stop the worker: crossing still works and Quit stops. Verify
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
