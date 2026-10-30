# Local keyboard remapping: safety design (5H)

Status: **5H-3 Windows integration implemented, COMPILE-CHECKED ONLY;
native Windows UNTESTED, EXPERIMENTAL, default OFF**. macOS native integration
is still missing (next slice), also **UNTESTED**. Linux local remapping is not
available. `[input] local_rules = false` remains the default.
Do not connect the existing asynchronous injection channel to a hook and
describe it as safe: enqueueing a release is not acknowledging its delivery.

## Ownership and acknowledgement

One serialized worker must own the mapper, replacement snapshots, tap/hold
deadlines, device generations, and the local injector. It must process capture
activation from **every** entry point (edge, switch, broadcast, physical hotkey)
through the same barrier before publishing `Activated` or forwarding input.
The hook must never wait for that barrier. While pending, capture activation
is deferred, not optimistically enabled.

The pure `Handoff` now exposes a shared `HookGate`: one `AtomicU8` read with
acquire ordering per decision. `begin` and `fail` publish the closed gate with
release ordering **before** running cleanup; only a current acknowledged
ticket publishes Local or Remote. Dropping the worker's `Handoff` closes any
surviving hook handle. Clone handles outside callbacks and borrow them there;
do not clone/drop the last reference in a hook. This supplies atomic routing
visibility, not a native queue fence: in-flight decisions still need serialized
queue admission, worker revalidation and acknowledged injection. Windows now
consumes this handle; dropping it alone still cannot release injected keys.

### Implemented Windows path (not native acceptance)

`windows_localmap.rs` connects the existing WH_KEYBOARD_LL hook to the shared
`localmap_runtime::{Physical, Admission, Engine}` production logic. The hook
borrows an immutable `arc-swap` snapshot; retired generations are retained and
reclaimed off-hook. `arc-swap` 1.9.3 and `crossbeam-queue` 0.3.14 are pinned by
Cargo.lock; neither is claimed to have undergone an independent security audit.
The bounded 64-entry admission ring reserves monotonically numbered work before
swallowing. The local mapper admission path does no mapper work, allocation,
channel send, file IO or mutex acquisition; its OS calls are deferred-window/event
wakeups. Snapshot reader TLS is warmed on the capture thread before publication.
The existing remote forwarding path is not covered by this allocation claim. A counting
allocator tests the production snapshot/decision/admission path, not Win32 or
native callback timing. The approximately 1 ms callback budget is **UNTESTED**.

Before a Remote fence the capture thread closes admission, drains the ring and
finishes the Raw Input correlator, including its delayed/unknown-device events.
The same worker consumes these numbered keys and then checks the exact queue
watermark, cancels pending dual roles, and releases all successfully injected
downs. SendInput's actual successful prefix updates a separate held ledger;
a partial release prevents acknowledgement and remains available for retry.
Only the matching successful fence completion permits `activate_now` and remote
forwarding. Return to Local uses another fence and an empty mapper. Settings
reload, edge activation and explicit capture commands use this path.

Only the serialized worker can restore health, after consuming the admitted
watermark without replaying failed work. Recovery compares the exact atomic
failure generation before the fence and after releasing outputs. A newer failure
(including one during release) invalidates recovery; a stale completion cannot
clear it. Capture-thread settings reload never marks the worker healthy itself.

The injector uses scancodes, E0 flags and the existing `dwExtraInfo` tag. Tagged
or LLKHF_INJECTED input and VK_PROCESSKEY bypass physical routing. Existing
AltGr fake-Ctrl decoding is retained; its interaction with local rules is
**UNTESTED**. Rules requiring virtual-key injection or actions are refused as a
whole snapshot; scancode keys, chords, sequences, `none`, layers and tap/hold
are supported. Physical emergency/custom shortcuts precede the mapper in both
local and remote paths. Emergency Esc passes, but earlier rewritten modifiers
are not retroactively delivered to Windows: OS receipt of the complete chord
is not promised. A swallowed Caps Lock does not deliberately inject its original
toggle; actual LEDs, IME and reserved combinations are **UNTESTED**.

Worker timers exist only for undecided dual roles. An independent event-woken
lease thread closes health/admission and queues serialized cleanup if numbered
work goes unacknowledged for 100 ms after observation; hook decisions also
enforce a 100 ms oldest-admission lease. This does not depend on the capture
message loop processing its own watchdog. The capture watchdog exists only
while work/fences are outstanding. Parked worker/lease threads do no periodic
work at idle; they remain allocated after first enable, even when disabled.
Release builds now unwind panics so worker catch-unwind/ledger cleanup can run;
this trades some binary size for recoverable worker failure. Abort, forced
termination and a blocked OS injector are outside that protection.

WTS session notifications, desktop-switch WinEvents and power broadcasts close
admission, release outputs and pause mapping. **Return/unlock/resume does not
automatically re-enable mapping**: once active on the Default desktop, explicitly
reload with `one-mk keymap local on`. Device-table replacement also pauses;
release all physical keys/buttons before resuming a paused mapper (otherwise
reload is refused with an inactive reason). The daemon supplies a fresh resolved
snapshot. Raw Input's correlated DeviceRef
is passed to `Mapper::key_from`; unknown input matches only base rules. Device
ids stop at exhaustion rather than wrapping. Hook ownership is necessarily by
physical code before correlation; overlapping same-code releases are retained
for worker routing, but multi-keyboard identity/ordering needs native acceptance.

`keymap local on|off` uses settings reload on Windows. Status, list/check and
doctor use backend acknowledgement (`active (N)` or an inactive reason), never
infer activity from the preference. During asynchronous reload, ask status again
after the fence. Doctor always warns that native Windows is UNTESTED.

`localmap_handoff::Handoff` models Local -> Handoff -> Remote and the reverse.
Starting a transition closes the local gate immediately and returns a unique
ticket. The worker cancels undecided tap/hold keys (no tap), calls release-all,
and submits the releases through the same ordered injector as replacements.
Only an acknowledgement **after successful injection of the entire batch**
may complete that ticket. Acknowledging an old ticket never opens a new gate.
Tickets are process-wide, never reused (including across worker instances);
exhaustion disables interception instead of wrapping. A new daemon must
discard the old worker and injector queues rather than replaying their tickets.
No command can cancel a pending barrier into Local or Remote without another
release acknowledgement. Failed or partial injection, timeout, session lock,
sleep, tap disable, device-table replacement, or daemon disconnect invalidates
the ticket and closes the gate. Re-entry requires a new barrier. An empty
batch still needs an acknowledgement of the ordered queue fence.

Remote has no local pending timer or emitted keys. Returning to Local starts
an empty mapper but retains physical ownership tombstones until key-up.
Device ids must be qualified by the device-table generation: never associate
a late release with a reconnected device that reused the same id.

## Held-key transition matrix

| Original down's owner | During barrier / after transition | Original key-up |
| --- | --- | --- |
| OS (passed locally) | Suppress repeats during Remote, never synthesize a new down | Pass to OS; never forward to peer |
| Local mapper | Release replacement before Remote acknowledgement; discard pending tap | Swallow original up; it has no OS down |
| Remote peer | Remote release-all before leaving Remote; do not repeat locally | Swallow original up; never inject a local down |
| No known down | Never invent a down | Pass in Local; remote capture applies its own ownership policy |
| Failed-open OS down | Retain OS ownership through recovery and rule reload | Pass; do not reinterpret under new rules |
| Session reset hides up | Release emitted keys; invalidate barrier and device generation | Resume only with an explicit fresh physical baseline |

Shortcut recognition may change the action on a **down**, never the ownership
of an **up**. Even if a native matcher flags every event of a chord, OS-owned
modifier releases always pass to the OS; local-owned releases drain locally
(or are swallowed after release-all); remote-owned releases reach the current
remote capture only, or are swallowed after that capture ends. Synthetic events
bypass this physical ownership table without changing it.

Queue overflow must happen **before** swallowing an original down. A swallowed
down must have a queued replacement, an explicitly disabled (`none`) rule,
an undecided tap/hold obligation, or a shortcut obligation. On overload all
new presses pass through. Already-owned ups are drained consistently. Strict
unconditional fail-open for already-swallowed releases is unsafe: it would
create unmatched OS ups. A lease-expiry path must distinguish these cases.

The pure runtime supplies the gate, bounded admission, physical routing and
worker ledger. Linux fake-hook/injector tests drive that same production logic;
they are not a native device correlator or Win32 injection test. Previously
mapped codes retain release-routing history for overlapping keyboards; stray
ups have no mapper output, and new failed-open downs keep OS ownership.

## Hook contract (Windows compile-only; macOS pending)

The hook reads only a preallocated immutable decision snapshot, physical
ownership bitsets, and atomics. No mapper allocation, mutex, channel allocation,
logging, I/O, destruction of the last snapshot reference, or blocking injector
call is allowed. Publish snapshots and reclaim them off the hook thread.
The existing `Mapper::key_from` can allocate a source: it belongs on the worker,
not inside the callback. Windows admits eligible physical scancodes first and
correlates on the capture loop before the worker evaluates device rules. An
unmatched admitted key is re-injected, rather than claiming the delayed correlator
can decide synchronously in the hook. Unknown/ambiguous devices use only
non-device local rules.

Physical emergency Ctrl+Alt+Shift+Esc and user hotkeys precede rule evaluation,
including rules for modifiers and Esc. On emergency, close the local gate,
release held replacements and return home. A physical matcher observing a
chord is not enough if earlier modifier downs were already rewritten: do not
claim the OS received the chord. Local `--app` remains unsupported because the
existing app probe describes a remote destination and cannot block a hook.

New presses fail open with an empty snapshot, disabled preference, disconnected
daemon channel, expired worker lease, unsupported scancode or inactive
session. A worker lease alone does **not** detect a hung daemon: native glue
also needs an event-driven daemon liveness acknowledgement. No periodic idle
heartbeat: arm watchdogs only while interception or a barrier has obligations.
Only undecided dual-role keys arm the tap/hold timer.

Windows: ignore `LLKHF_INJECTED` and the existing `dwExtraInfo` tag; pass
`VK_PROCESSKEY`; retain AltGr's synthetic Ctrl/RAlt semantics. Inject physical
scancodes with the correct E0 flag (right Ctrl/Alt, arrows, numpad). Swallow a
remapped Caps Lock original so its toggle/LED is not changed. Monitor session,
desktop and power notifications and suspend until the session is active.
Low-level hooks may be silently removed on timeout: no blocking work there.

macOS: ignore the existing event-source user-data tag; on
`kCGEventTapDisabledByTimeout` or `kCGEventTapDisabledByUserInput`, close the gate
and release mapped output before enabling interception again. Re-enabling the
tap is not proof the old mapper was drained. Session and sleep notifications
use the same barrier. Permission loss remains fail-open and visible.

## Scope and persistence

Local+Device > Local; remote/global/OS/peer rules never affect local typing.
Store fingerprints in this computer's settings, not ephemeral device ids.
Unknown devices never match device-specific rules. Mouse buttons, local pointer
speed, and local app rules are outside 5H. Existing files keep their meaning.
The default must stay off until real-hardware acceptance. Never label stored
rules `active` merely because a preference is enabled: require a backend
acknowledgement of its snapshot generation and a healthy gate.

## Failure boundaries and acceptance

User-mode hooks do not rewrite DirectInput/Raw Input readers, secure desktops,
Ctrl+Alt+Del, lock screens or UAC. Win+L and other reserved combinations cannot
be promised to obey remaps. No in-process design can guarantee key-up after
`kill -9` between successful SendInput/CGEventPost down and release; an
independent recovery process or OS-tested recovery strategy is required before
claiming this guarantee. Do not claim the pure tests cover native injection,
AltGr, Caps Lock LEDs, IME, permissions or hook timeouts.

Native acceptance (Windows and macOS separately, currently **not passed**):

1. Use a disposable login and keep another computer connected for recovery.
   Record the original settings; never overwrite another computer's settings.
2. On Windows only, enable `one-mk keymap local on`, add
   `one-mk keymap add capslock esc --local`, then add a device override with
   `--local --device NAME`. Check list/check/status/doctor and both keyboards.
3. Hold remapped, passed, dual-role and modifier keys across edge, switch,
   broadcast and hotkey transitions in both directions; check every matrix row.
4. Test physical emergency and custom shortcuts with Esc/modifier mappings;
   repeat, AltGr, sided modifiers, arrows/numpad, IME and Caps Lock toggle/LED.
5. Lock/unlock, secure desktop/UAC (Windows), sleep/resume, unplug/replug,
   permission loss and tap timeout (future macOS slice); kill the worker and daemon while
   keys are down, including `kill -9`. Check actual OS state, not only logs.
6. Measure idle CPU and resident memory with feature off/on and no held keys.
   Observe that no periodic timer remains armed at idle. After each resume,
   confirm inactive status, release physical keys, then explicitly reload.
7. Recovery: physical Ctrl+Alt+Shift+Esc; from the other computer run
   `one-mk keymap local off` on the affected host. If the daemon is dead,
   disable its autostart and restore the recorded settings before restarting.
   If injection was denied by UIPI or the worker is stuck in an OS call,
   turning the preference off cannot itself prove releases were delivered.
   Release/tap the affected physical modifiers; sign out or reboot if needed.
8. If this experimental build prevents normal login recovery, use Windows
   Recovery Environment → Startup Settings → Safe Mode, or macOS Safe Mode
   (hold the power button on Apple silicon, choose the startup volume while
   holding Shift; hold Shift during startup on Intel). Disable one-mk's
   autostart, set `[input] local_rules = false` in the affected account's
   configuration, and reboot normally before restarting one-mk. Do not enable
   the experimental feature again until the failure is understood. These
   platform recovery steps are **UNTESTED** here. Never test in a real session
   during automated development; use an isolated disposable Windows login.

Windows manual command script (**UNTESTED**, disposable account only):

```powershell
one-mk keymap local off
one-mk keymap add capslock esc --local
one-mk keymap check
one-mk keymap local on
one-mk keymap list
one-mk status
one-mk doctor
# Execute matrix steps 3-6; after unlock/resume, verify inactive before reload.
one-mk keymap local on
one-mk status
# Recovery and end of test; restore the original settings afterwards.
one-mk keymap local off
one-mk status
```

Residual release blockers: native same-code multi-keyboard correlation,
Raw Input delivery of swallowed originals, UIPI/secure-desktop release denial,
OS-hook removal/latency, process termination and worker blocked in SendInput.
An idle daemon stall is not detected by an idle heartbeat (there is none).
Native CPU/RSS/latency and Windows session/power behavior remain unmeasured.
These are not an enterprise no-stuck-keys acceptance claim. macOS needs its own
tap/injector/session implementation and separate native acceptance.

### 5H-3 validation and hostile review (2026-10-30)

Clean-environment Linux workspace tests and workspace all-targets clippy are the
automated acceptance checks, alongside all-targets cross-clippy for
`one-mk-input`, `one-mk-proto`, `one-mk-ctl`, `one-mk-app`, and `one-mk-tray` on
`x86_64-pc-windows-msvc` and `aarch64-apple-darwin`. All cargo commands use `-j 2`.
No native hook was installed or executed. The extra Windows **daemon** (`one-mk`)
cross-check is blocked before Rust checking by `ring` needing unavailable
`lib.exe`; its Windows-only settings dispatch has not been cross-checked here.
The extracted settings/snapshot builder is compiled and tested on Linux.

The production hook/admission/worker harness replays randomized key streams,
local/remote fences, failure/pause cleanup and recovery; separate device-source
tests balance overlapping keys. Regression coverage includes partial injection,
exact fence watermarks, emergency physical chords with rewritten modifiers/Esc,
dual-role cancellation, independent idle watchdogs, unwind cleanup, failed queued
work and recovery-generation races. The allocator test exercises 100,000
production snapshot/admission down/up pairs with zero allocation/deallocation
(plus the existing exhaustive decision matrix); this is not a native timing test.

Hostile review found and fixed capture-thread premature health recovery,
ordinary-reload baselines incorrectly sampling injected modifiers, session
notification registration failure, and watchdog dependence on a blocked capture
loop. Four manual mutations were rejected by regression tests: bypassing the
queue watermark, forgetting unsuccessful releases, swallowing emergency Esc,
and removing the post-release failure-generation comparison. All were reverted.
Remaining native acceptance blockers are listed above; do not mark 5H complete.

### Review regression coverage (5H-2 prerequisites, not native activation)

The release oracle is an explicit ownership/state table, independent of the
decision implementation, including chord-marked key-ups and both pending
destinations. A two-thread test blocks cleanup to check that the hook already
sees a closed gate, then checks visibility of injection results through the
acknowledged gate's release/acquire publication. Worker exit closes surviving
handles. The counting allocator covers allocation, zeroed allocation,
reallocation and deallocation while exercising all hook decision flags. None
of these tests installs a hook, proves native delivery or measures hook latency.
Manual mutation checks separately restored the old shortcut-up override,
removed closure before cleanup, and removed closure on worker exit; each was
rejected by its corresponding regression test. All mutations were reverted.
