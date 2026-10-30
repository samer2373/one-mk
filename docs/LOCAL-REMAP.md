# Local keyboard remapping: safety design (5H)

Status: **incomplete; inactive on every backend**. Windows and macOS are
**experimental, untested on real hardware**. Linux local remapping is not
available. The pure handoff model is a prerequisite, not a native activation.
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
queue admission, worker revalidation and acknowledged injection. No platform
currently consumes this handle, and dropping it cannot release injected keys.

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

The current pure API supplies the gate and routing policy only. The caller
must retain per-device physical ownership and retire mapper bookkeeping on
swallowed key-ups. The Linux fake harness demonstrates this ordering; it is
not a production native worker, device correlator or injector.

## Hook contract (not yet implemented)

The hook reads only a preallocated immutable decision snapshot, physical
ownership bitsets, and atomics. No mapper allocation, mutex, channel allocation,
logging, I/O, destruction of the last snapshot reference, or blocking injector
call is allowed. Publish snapshots and reclaim them off the hook thread.
The existing `Mapper::key_from` can allocate a source: it belongs on the worker,
not inside the callback. Device correlation must be available **before** the
callback decides; the remote correlator's delayed event output is not a local
hook decision. Unknown/ambiguous devices use only non-device local rules.

Physical emergency Ctrl+Alt+Shift+Esc and user hotkeys precede rule evaluation,
including rules for modifiers and Esc. On emergency, close the local gate,
release held replacements and return home. A physical matcher observing a
chord is not enough if earlier modifier downs were already rewritten: do not
claim the OS received the chord. Local `--app` remains unsupported because the
existing app probe describes a remote destination and cannot block a hook.

New presses fail open with an empty snapshot, disabled preference, disconnected
daemon, expired worker lease, unavailable device correlation or inactive
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
2. Once native activation exists, enable `one-mk keymap local on`, add
   `one-mk keymap add capslock esc --local`, then add a device override with
   `--local --device NAME`. Check list/check/status/doctor and both keyboards.
3. Hold remapped, passed, dual-role and modifier keys across edge, switch,
   broadcast and hotkey transitions in both directions; check every matrix row.
4. Test physical emergency and custom shortcuts with Esc/modifier mappings;
   repeat, AltGr, sided modifiers, arrows/numpad, IME and Caps Lock toggle/LED.
5. Lock/unlock, secure desktop/UAC (Windows), sleep/resume, unplug/replug,
   permission loss and tap timeout (macOS); kill the worker and daemon while
   keys are down, including `kill -9`. Check actual OS state, not only logs.
6. Measure idle CPU and resident memory with feature off/on and no held keys.
   Observe that no periodic timer remains armed at idle.
7. Recovery: physical Ctrl+Alt+Shift+Esc; from the other computer run
   `one-mk keymap local off` on the affected host once that command is
   available. If the daemon is dead, disable its autostart and restore the
   recorded settings before restarting. This revision has no native hook to
   recover. The on/off command currently stores only a preference and reports
   that native activation is incomplete.
8. If a future native build prevents normal login recovery, use Windows
   Recovery Environment → Startup Settings → Safe Mode, or macOS Safe Mode
   (hold the power button on Apple silicon, choose the startup volume while
   holding Shift; hold Shift during startup on Intel). Disable one-mk's
   autostart, set `[input] local_rules = false` in the affected account's
   configuration, and reboot normally before restarting one-mk. Do not enable
   the experimental feature again until the failure is understood. These
   platform recovery steps are **UNTESTED** here; this revision still installs
   no native local-rule interceptor.

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
