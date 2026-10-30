# Hostile review: 5H local remap, native Windows + macOS (wave5)

Reviewer: Claude (independent). Scope: docs/LOCAL-REMAP.md, crates/input/src/{localmap_runtime,localmap_handoff,windows,windows_localmap,win_logic,macos,macos_localmap,macos_localmap_logic,macos_devices,devcorr}.rs, proto remap Mapper parts, daemon reload/status/doctor glue, Cargo profile.
Method: read as a compiler/OS would. Ran `cargo check -p one-mk-input --target x86_64-pc-windows-msvc --target aarch64-apple-darwin` (clean env): passes. No native code executed.

Ranked findings (severity, location, scenario, fix).

---

## 1. HIGH (regression for every Windows user, feature off): any WM_POWERBROADCAST ends remote control
windows.rs:1180-1187

```rust
WM_APP_LOCAL_DESKTOP | 0x218 => {
    ... state.local_pause(); state.end() ...
```
`0x218` (WM_POWERBROADCAST) is matched for **every** wParam, and `state.end()` runs whether or not local rules exist. The capture window is a top-level WS_POPUP window, so it receives broadcasts such as PBT_APMPOWERSTATUSCHANGE (AC plug/unplug, battery thresholds and percentage steps).
Scenario: a laptop user controlling another computer plugs in the charger, or the battery drops a few percent. Capture ends and the pointer returns home. This happens without local rules.
Fix: only act on PBT_APMSUSPEND (4) and the resume codes (7, 18), and call `local_pause` only there. Do not call `end()` for anything else; resume can still end capture as before. Return TRUE only for the handled codes.

## 2. HIGH (Windows): edge crossing, `one-mk switch` and switch hotkeys are refused after common events, even after the feature is turned off
windows.rs:799-809 (`activate`), localmap_runtime.rs:106-107, windows.rs:660-665, windows_localmap.rs:210-217

`activate()` returns `(false, Effect::None)` and sends no answer when `local.paused || !local.admission.healthy()`. The `Local` instance is created on the first enable and is never dropped, so this also applies after `keymap local off`.
- Every one-mk hotkey pressed at home goes through `Physical::shortcut`, which calls `admission.fail()` on every `Fire`. The resulting `Event::Hotkey(Cycle/Jump)` leads to `CaptureCmd::Activate`, and `activate()` refuses it because health is now odd. **Switch hotkeys never work while local rules are active**: RegisterHotKey cannot fire either, because the physical matcher swallowed the trigger key. The daemon's switch times out quietly.
- Lock/unlock, any UAC or secure-desktop switch, and sleep/resume set `paused = true` and nothing clears it, so the mouse can no longer cross to any peer. With the preference off this lasts until the next settings reload or device-change reload; with it on, until an explicit reload.
- If the worker panics, health stays failed forever, so crossing is lost until the daemon restarts.
Fix: never let the experimental worker gate core capture. If the gate is closed and the release ledger is empty, activate directly. Drop or restart `Local` when the preference is off. Do not fail admission for non-emergency hotkeys: close the gate and run a fence instead, and recover after it.

## 3. HIGH (macOS): crossing needs a successful fence, so a dead worker or missing receipts block it permanently
macos.rs:888-896, 898-903; macos_localmap.rs:285-292; localmap_runtime.rs:384-391

`start()` always fences to Remote. The fence calls `Engine::fail()`, which must deliver every release in the ledger with a receipt. A release whose receipt never arrived stays in `held` forever.
- Receipts not observed at the HID tap (documented as UNTESTED), Secure Input, or a lost `try_borrow`: after the first remapped key, every fence fails and the Mac can never cross again, even with `keymap local off` (that is also a fence).
- Worker panic: `work.send` fails, then `fail()` runs, `pending = None` and `activation = None`. Each edge hit retries and fails. In addition, `Msg::Quit` calls `local_fence`, the send fails, nothing stops the run loop, and the capture thread hangs at shutdown.
Fix: as in #2, capture must not depend on ledger acknowledgement. Bound the fence: after N failed retries, give up on the ledger, report it, and allow capture. Stop the run loop on Quit when the fence cannot be submitted.

## 4. HIGH (Windows): after a remote visit with `enabled == false` the gate stays Remote and key auto-repeat at home is swallowed
windows.rs:771-773; localmap_handoff.rs:175-177; localmap_runtime.rs:117-139

`activate()` fences to **Remote** whenever `Local` exists. `release()` fences back to **Local** only if `local.enabled && !local.paused`. `enabled` is false when the preference is turned off after having been on, when there are zero local rules, or when the snapshot is unsupported (for example a rule that outputs a media key). In those cases the gate stays `Remote` after returning home. In the local hook a new down then routes `Route::Remote`, so it passes but is recorded in `passed` (OS-owned). Each auto-repeat down is `Owner::Os` with gate Remote, which returns `Route::Suppress`.
Scenario: the user turns local rules off, visits a peer, comes back, and holding Backspace deletes one character. This lasts until the next visit or reload.
Fix: always fence (or close the gate) on `release()` when `Local` exists. Better, drop `Local` when it is disabled.

## 5. HIGH (Windows): `Physical.mapped` is never cleared, so key-ups whose down the hook never saw are swallowed and modifiers stick
localmap_runtime.rs:119, 129-133

`owner = ... || !key.down && self.mapped.is_held(code) => Owner::Local`. `mapped` is set on every admitted down (which means every eligible key ever typed) and never cleared. An up whose down was not seen becomes Owner::Local. If the gate is closed or paused it is `Suppress`. If it is admitted, the mapper has no source for it, so nothing is emitted. Either way the OS never gets the up.
Scenario: Alt+Tab away from an elevated window. The low-level hook is not called while an elevated window is in the foreground; the comment at windows.rs:1063 states this. Alt down goes to the OS unseen, Tab switches to a normal window, and the hook then sees Alt up, which is swallowed. **Alt stays stuck in the Windows key state**: the next keypress activates menu accelerators and clicks become Alt-clicks. The same happens with Alt+Y or Enter on a UAC prompt: the down happens on the secure desktop and the up arrives after the desktop switches back. On main, unknown ups always passed. This is a regression.
Fix: keep a per-code count of admitted downs that are still outstanding, cleared on up and on fence. Let an up with no current owner pass (Owner::Unknown). Add a regression test: an unseen down followed by a seen up must pass.

## 6. HIGH (Windows): a release that fails on the secure or lock desktop is never retried, and recovery is blocked by the stuck key
windows_localmap.rs:210-217; windows.rs:1172-1176, 1009-1015, 1324

On a desktop switch, WTS lock or power event, `local.fail()` sends `Work::Fail` once (`if !self.failed`). The worker's SendInput of the releases runs while the Winlogon/secure desktop is the input desktop, so it fails and the codes stay in `held`. When the user returns, the second EVENT_SYSTEM_DESKTOPSWITCH or the unlock does **not** retry, because `failed` is already true. The stale `owned` tombstone for the physical key then swallows its next press while paused.
Scenario: with `capslock -> lctrl`, the user presses Ctrl(Caps)+Shift+Enter in Start search to run as administrator. The UAC secure desktop appears while the injected LCtrl is down. The LCtrl release fails. After UAC, **Ctrl is stuck**. `one-mk keymap local on` is then refused ("release all physical keys"), because `async_held()` (GetAsyncKeyState) reports the stuck *injected* LCtrl. Only `keymap local off` or a physical LCtrl press clears it.
Fix: retry the release ledger automatically on WTS_SESSION_UNLOCK and on the desktop switch back to Default (send Work::Fail again or fence to a closed state). Exclude keys present in the worker ledger from the "physical keys held" check. Clear `owned` tombstones for codes that GetAsyncKeyState shows are not down.

## 7. HIGH (macOS): phantom modifier flags are added to every event after a missed modifier release, even when local rules are paused or off
macos_localmap_logic.rs:50-55; macos_localmap.rs:259-271; macos.rs:820-830

`Flags::apply` removes all modifier bits from the real event and rebuilds them from `Flags.physical`, which tracks the flagsChanged events the tap has seen. This runs for every key and pointer event whenever `Local` exists, whether paused or disabled. If a modifier up is missed (tap disabled by timeout or user input, Secure Input, tap re-enable), `physical` keeps that modifier, and the flag is added to every later key and mouse event until the same key is pressed again.
Scenario: Cmd+Tab into Terminal with Secure Keyboard Entry, or a tap timeout under load while Shift is held. Afterwards clicks are Cmd-clicks and typing triggers Cmd shortcuts.
Fix: never add a physically tracked bit that the original event lacks: `passed_bits & original`, with device-bit granularity. Resync `Flags` and `Physical.down` from `CGEventSourceFlagsState(HIDSystemState)` after a tap re-enable or the end of Secure Input, and on baseline. Skip rewriting when the gate is closed and no output is held.

## 8. MEDIUM-HIGH (both): any one-mk hotkey turns local rules off until a manual reload
localmap_runtime.rs:106-107; macos.rs local_pump (`action.is_some()` leads to `local_pause`); windows.rs:660

Every shortcut `Fire`, not only the emergency chord, fails health. On macOS `local_pump` pauses ("physical shortcut; release all keys and reload"). On Windows `local_pump` calls `local.fail()` (plus #2). A user who presses a clipboard-fetch, lock-pointer or broadcast hotkey silently loses their remaps.
Fix: for non-emergency actions, swallow the trigger and record ownership without failing health. Keep the fail and release-all only for the emergency chord.

## 9. MEDIUM (both): the fence discards the mapper's tombstones, so held keys re-press and dual-role keys emit a spurious tap
localmap_runtime.rs:386-387 (`self.mapper = Mapper::with_set(...)` right after `fail()`/`release_all`)

`Mapper::release_all` marks physically held keys `Rec::Swallow` ("do nothing until they come up"). Every Windows and macOS fence passes `Some(rules)`, so the mapper is replaced and those tombstones are lost. The hook still owns the key (`owned`), so its auto-repeat downs are admitted to the fresh mapper as a **new press**.
Scenario: CapsLock is tap Esc / hold Ctrl and is held when a reload happens (settings, trust or device-change auto-reload) or while returning from Remote. The next repeat starts a new pending dual role, and the release resolves as a tap, which emits a **spurious Esc**. Plain remaps emit a fresh down after the release.
Fix: call `release_all` and then `set_keymaps` on the existing mapper, or seed the new mapper with Swallow records for `Physical.owned` codes. Add a test with an auto-repeat across a fence.

## 10. MEDIUM (Windows): hook blindness under an elevated foreground window leaves replacement output stuck
windows.rs (no local equivalent of `foreground_elevated`)

The low-level hook is not called while an elevated window is in the foreground. Holding a remapped key (CapsLock→LCtrl) while focus moves to an elevated window means the physical up is never seen and the injected LCtrl stays down. SendInput into the elevated foreground is also UIPI-blocked. The lease watchdog does not fire, because nothing is outstanding.
Fix: watch EVENT_SYSTEM_FOREGROUND (out-of-context WinEvent). When the new foreground is elevated and we are not, pause and release before UIPI applies, or refuse local rules unless elevated. Document it in the acceptance matrix.

## 11. MEDIUM (global): `panic = "unwind"` across the whole workspace removes fail-stop behaviour
Cargo.toml:13

The change is made to let the worker's catch_unwind release keys. It also changes every daemon thread on all OSes, Linux included. Panics in tokio tasks are now caught silently and the task dies, while the daemon keeps running half-broken; before, it aborted and autostart restarted it. Panics in Rust code under `extern "system"` callbacks still abort. The binary also grows, against the tiny-footprint goal.
Fix: restore `abort`. Do a best-effort release from a `std::panic::set_hook` that reads the ledger (a lock-free snapshot of held codes), or move injection behind a supervisor.

## 12. MEDIUM (both): resuming refuses while any key or button is held, which races with the Enter key that runs the command
windows.rs:1009-1015; macos.rs LocalRules handler

`one-mk keymap local on` runs on Enter down. The settings apply and IPC take milliseconds, so the reload usually lands while Enter is still physically down, and it is refused with "release all physical keys/buttons". On macOS the check ignores `rules`, so even `keymap local off` is refused and the off-preference is not applied.
Fix: baseline held keys as OS-owned (`Physical::baseline` already supports this) instead of refusing, or wait for the releases. Never refuse a disable.

## 13. MEDIUM (macOS): NSWorkspace sleep/wake/session observers probably never fire, so the docs overclaim
macos.rs:399-414

The observers are registered with `queue: None` on the capture thread. NSWorkspace's notification center delivers on the main thread and relies on the main run loop, and the daemon's main thread runs tokio (cursoroverlay_macos.rs:15 says so). In that case `Msg::Session` never arrives for sleep, wake or fast user switching. The 1 s tick and the watchdog only check while something is pending or held. LOCAL-REMAP.md claims workspace changes and sleep/wake close admission.
Fix: use IORegisterForSystemPower plus CGSession polling on the capture run loop, or run these observers on a thread whose run loop is guaranteed, and verify natively. Correct the docs.

## 14. MEDIUM (macOS): a tap disabled by timeout or user input now ends remote control for all users
macos.rs:790-797

`self.end(&mut st)` is unconditional, so it also runs when no local rules exist. Before, macOS just re-enabled the tap. Under load, tap timeouts now kick users back home.
Fix: only call `end()` when local rules hold outputs, or keep the old behaviour when `local_rules.is_none()`.

## 15. LOW-MEDIUM (Windows): starting while a mouse button is held disables mapping indefinitely
windows.rs:1016-1018; localmap_runtime.rs:84, 122

`async_held()` includes BTN_LEFT and the other buttons. On start, `baseline` puts them in `passed`. Mouse ups never reach the keyboard hook, so `passed` is never cleared, `eligible = ... && self.passed.is_empty()` is always false, and nothing is remapped. The status still says `active (N)`.
Fix: baseline only keyboard codes.

## 16. LOW-MEDIUM (macOS): a stale `physical_held` makes the watchdog poll at 10 Hz forever
macos_localmap.rs:189-206, 241-242

A key-up hidden by Secure Input or a tap disable leaves `Local.down` non-empty. `Local::fail` clears `physical_held`, but the next key event sets it again from the stale `down`. The watchdog then calls `park_timeout(LEASE)` indefinitely, which breaks the zero-idle-CPU goal.
Fix: reset `down` on fail/pause, or resync it from `CGEventSourceKeyState`.

## 17. LOW (both): one sequence gap wedges the engine until the daemon restarts
localmap_runtime.rs:347-352, 386; windows_localmap.rs:201-208; macos_localmap.rs:118-126

On a mismatch `forward` drops the popped key. `Engine::key` then fails without advancing `sequence`, every later key fails, and `fence` requires `watermark == sequence`, so it fails forever. Because of #2 and #3, capture is also lost.
Fix: let a fence resynchronise `sequence = watermark` after a drain (the ledger is already released).

## 18. LOW (Windows): latency and reordering against passthrough events
windows.rs:614-623, 725-733; devcorr.rs:24

Every eligible key goes hook → correlator (waits up to 8 ms) → WM_TIMER (about 15.6 ms granularity at home; timeBeginPeriod is set only while captured) → worker → SendInput. Ineligible or unmapped events pass immediately and overtake earlier keys: AltGr's fake LCtrl, numpad fake-shift, NumLock/Pause/media keys, mouse clicks. Example: Ctrl followed quickly by a click arrives as a plain click.
Fix: skip the correlator wait when the snapshot has no device rules, and use a 1 ms high-resolution timer only while local work is pending.

## 19. LOW: inconsistent "physical hotkeys only"
On Windows, RegisterHotKey fires on injected input, so mapper output (CapsLock→LCtrl, plus Alt+Right) triggers a Ctrl+Alt+Right one-mk hotkey. On macOS the shortcut tap ignores receipt-tagged output. Decide on one behaviour and document it.

## 20. LOW (macOS, verify natively): re-posted keys lose the original keyboard type
macos_localmap.rs:52-72

`CGEventSource::new(Private)` with `new_keyboard_event` does not carry `KeyboardEventKeyboardType` from the physical event. On ISO Macs this is the classic §/` swap risk.
Fix: carry the keyboard type in `Key` and set it on the posted event.

## 21. LOW: noise for users who never enabled the feature
Every Windows/macOS daemon sends `Status{"local-key-rules", Some("Local key rules inactive: preference off; EXPERIMENTAL ...")}` (an input "problem" line in `status`), and `doctor` always WARNs on Windows/macOS. Use `problem: None` when the preference is off.

## 22. LOW: doc/test accuracy
- The docs say "Return/unlock/resume does not automatically re-enable mapping". In fact any `Event::Devices` triggers `reload_local_rules` (app/tune.rs:245), which auto-resumes on both OSes when nothing is held (for example after wake, when keyboards re-enumerate). On macOS, `Hid::devices` (`Dispatch::fail`) does not even set `paused`.
- The allocation test (tests/localmap_decision.rs) uses `Rules::default()` (no hotkey bindings), so the matcher with bindings is not covered.
- `macos_modifier_streams_tap_disable_and_secure_input_use_the_production_gate` models neither a tap disable nor Secure Input (it only calls `admission.fail`). Its final `flags.apply(0, 0) == 0` cannot fail, because the original is 0.

---

## Verified OK
- The input crate cross-checks for x86_64-pc-windows-msvc and aarch64-apple-darwin (cargo check passes). The daemon's Windows/macOS-only call sites (app/tune.rs reload_local_rules, doctor macos_permissions, CaptureCmd::LocalRules cfg) match the input crate's signatures and fields; I found no compile errors by reading them.
- The emergency Ctrl+Alt+Shift+Esc chord is evaluated on physical `Physical.down` in the hook before routing. It is not delayed or remapped (Esc passes, Esc up is OS-owned) and is not affected by the matcher's `busy` state.
- New presses fail open on unhealthy/lease/overflow (`admit` fails before swallowing). An owned up is swallowed only when its down was swallowed (except #5).
- HookGate release/acquire publication; Feedback generation tokens (odd = failed; a stale recovery cannot clear a newer failure); ticket monotonicity.
- Windows: the hook bypasses OUR_INPUT/LLKHF_INJECTED/VK_PROCESSKEY before borrowing state. SendInput is called one INPUT at a time to get an exact successful prefix. KEYEVENTF_SCANCODE|EXTENDEDKEY is used for E0 keys. Pause/NumLock/media keys are refused (ineligible or unsupported). The AltGr fake Ctrl (scan&0x200) is passed untouched. Snapshot readers never drop the last Arc on the hook thread (same-thread replace and retire). Wake event lifetime is correct. Watchdog/worker shutdown order is correct.
- macOS: receipt arm/observe CAS only accepts the outstanding sequence. An uncertain down counts as held and an uncertain up stays in the ledger. Receipt-tagged output bypasses both taps. Caps Lock, Fn, F-keys and media keys are refused as sources and outputs. The tap callback catches panics (`C-unwind`).
- Local-scope rules are excluded from remote resolution; `--app` local rules are rejected.
