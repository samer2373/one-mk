# Keyboard layouts and AltGr (Wave 5E)

## Status and safety boundary

This is an **incomplete implementation**, not a claim of cross-layout support.
The default remains physical keys. The typing preference, validated wire types,
pure routing state machines and a dlopen-based XKB resolver/reverse lookup are
implemented; native layout discovery,
composition, shortcut reverse lookup, and native text injection are not yet
connected. The XKB resolver is exercised against system keymap data, not the
live desktop. Native backends therefore continue sending physical keys, even when
characters or auto is requested. Status, doctor, and the CLI must say so.
Do not advertise TEXT until a backend can receive text correctly. No polling or
background timer is introduced by the typing preference or routing engine.

## Problem and alternatives

An evdev key identifies a position, not a letter. German Z and US Y share a
position; French AZERTY, Dvorak and Turkish letters expose the same distinction.
Right Alt is often AltGr, not a shortcut Alt; Windows may also synthesize Ctrl
with AltGr. A shortcut must preserve the intended letter, whereas ordinary
typing must preserve the layout's produced text. Native IMEs complicate this
further. A layout name alone does not describe lock state, group, custom maps,
or the desktop's composition state.

Options considered:

- Always physical: compatible, useful for games, wrong for different layouts.
- Always Unicode: correct letters but loses shortcuts and held-key semantics.
- Clipboard paste: changes the user's clipboard, leaks data and is rejected.
- Hard-coded national tables: inaccurate variants/custom layouts; test fixtures
  only, never a substitute for querying the sender's OS.
- Hybrid: default physical, opt-in text plus character-aware shortcut keys.
  This is the chosen design, gated on real backend support.

## Preferences and negotiation

`one-mk typing --keys physical|characters|auto [--peer ID]` saves `[typing]`
in settings.toml. No arguments prints the preference and effective fallback.
Peer selectors accept an unambiguous paired name or fingerprint prefix; stored
overrides use the full fingerprint, not display names. Auto requires
descriptors from both ends and proven resolution/injection support; missing or
equal descriptors mean physical. Descriptors are informational, never shell
commands or filenames to open. A custom map must not be inferred from a name.
Preferences affect outgoing typing. The receiver accepts text only during the
authenticated controlling visit, subject to its supported backend and rate
limit once native routing is integrated. In this build text is never injected.
A future preference change must take effect on the next visit, never mid-hold.

## Proposed additive protocol-2 wire format (implemented codec)

All fields are little endian, inside the existing u16-length-prefixed control
frame. Existing `Key` tag 5 is unchanged. TEXT is capability bit 64, reserved
but deliberately absent from LOCAL until native support is ready.

| Tag | Fields | Rules |
| --- | --- | --- |
| 21 Text | u16 byte length, UTF-8 bytes | 1..8 bytes, exactly one extended grapheme cluster |
| 22 CharacterKey | u32 evdev code, u8 down (0/1), u32 Unicode scalar | code 1..767; unmodified printable single scalar; repeat down uses original resolution |
| 23 KeyboardLayout | four u16-length-prefixed UTF-8 strings: name, variant, rules, model | each <=128 bytes, no controls or format characters |

Only send these messages to TEXT-capable peers. Unknown tags remain skippable
by old peers, but capability checks prevent losing keys. Text allows Unicode
letters, marks, numbers, punctuation, symbols and ordinary U+0020 space. It
rejects all Cc/Cf/Cs/Co/Cn, separators other than space, bidi overrides, newlines,
Tab, Enter, Escape and Unicode line/paragraph separators. No format characters
(including ZWJ/ZWNJ) are currently allowed: this intentionally excludes some
emoji and language clusters. This is a conservative, documented limitation.
Text is not a command boundary: printable text can still spell shell commands;
only paired, authorized controllers may inject it. Never claim filtering makes
an untrusted controller safe. Decode rejects malformed UTF-8 and trailing bytes
on new messages. The pure receiver budget permits 200 text messages per fixed
one-second window, discards excess and has no idle timer; every repeat costs
one token. This is not a sliding-window limit: adjacent windows can admit two
bursts close together. The budget is not connected to the receive path yet.
Outbound invalid Text and descriptors encode an invalid empty value, not a
truncated usable character or layout, and remain bounded even for oversized
internal strings. Decoding refuses those empty values.

## Routing and held-key invariants

Physical local hotkeys run first; `Mapper::key_from` runs next. Resolve its
output with the sender's actual layout before Ctrl/Cmd swapping. Shift and
AltGr select text levels. Ctrl, ordinary Alt, and Meta select shortcut routing;
the Windows synthetic Ctrl+AltGr case must be identified by the backend, not
guessed from generic modifier bits. Modifiers remain physical. An unsupported
resolution falls back to physical for that press and retains that decision
until release. Never suppress a physical up for a physical down.

Text repeats send another Text without an up. Its source key up is suppressed.
Dead keys require a stateful OS/compose resolver; unavailable composition falls
back physically for the whole composition, not a mixture of text and dead keys.
CharacterKey includes the unmodified sender letter. The receiver resolves that
letter and required Shift/AltGr state once, remembers the resolved chord until
up, and uses reference counts for shared target codes/modifiers. Visit end,
disconnect, revoke, timeout and backend loss release all remembered keys. A
layout change during a hold must never change the up code. The pure engine
tests this; it is not yet wired into daemon visits.
Source codes are bounded to the evdev range so the pure engine can remember at
most 767 source keys. Reverse-lookup chords accept only Shift/AltGr modifiers;
invalid resolver output falls back physically rather than injecting arbitrary
modifiers. The existing per-device mapper must merge its output before this
computer-wide typing state machine; it is not a replacement for device pairing.

One layout per computer is the baseline. Device IDs remain attached through
mapping, but per-keyboard layouts exist only when the sender OS exposes them.
Japanese kana/IME composition and remote IME state are out of scope; use
physical mode with a matching receiver layout.

## Native implementation requirements (not yet implemented)

- Linux: dlopen libxkbcommon as in input/src/xkb.rs, no link-time dependency.
  Use the real keymap, current group, locks and modifiers. X11 must subscribe
  to XKB map/state changes, not assume `_XKB_RULES_NAMES` is authoritative.
  Portal/libei and wlroots must propagate their seat keymaps to resolution.
- X11 injection: prefer an existing keysym mapping. Temporary spare-keycode
  remapping requires a durable original-map journal, synchronization with other
  clients and restoration on every error and at startup. RAII alone cannot
  survive SIGKILL. Do not ship a remapping injector until SIGKILL recovery is
  exercised on a private X server. Never alter the owner's live map.
- Portal: NotifyKeyboardKeysym where actually supported; wlroots: safely
  regenerated virtual keymap or explicit physical fallback.
- Windows: ToUnicodeEx with non-state-mutating flags and the foreground hook
  HKL; KEYEVENTF_UNICODE for text, VkKeyScanEx for reverse lookup. Dead-key and
  synthesized Ctrl handling need native tests.
- macOS: UCKeyTranslate/CGEventKeyboardGetUnicodeString for resolution,
  CGEventKeyboardSetUnicodeString for injection, reverse lookup on the current
  input source. Permission loss must release the resolved held chords.
- Null: explicit test layout/injector only. It must not claim native support.

## Verification of this partial patch (October 30, 2026)

All test processes and real binary runs used a clean environment, private Xvfb,
throwaway HOME/runtime/settings, no WAYLAND_DISPLAY and no
DBUS_SESSION_BUS_ADDRESS. Nothing was injected into the live desktop. No native
typing injector, two-daemon cross-layout scenario, or keyboard-map crash
restoration was tested: those paths do not exist in this patch.

- Workspace tests: 875 passed, one failed, seven ignored (including doc tests).
  The only failure was the known clean-environment portal reconnect test,
  `portal::tests::the_portal_going_away_lets_go_and_reconnects_when_it_is_back`,
  timing out at `crates/input/src/portal_tests.rs:708`. Portal source was not
  changed. The workspace suite is therefore **not green**.
- Linux workspace clippy and the requested Windows x86_64-pc-windows-msvc and
  macOS aarch64-apple-darwin package/all-target checks passed with `-D warnings`.
  Windows and macOS were **compile-checked only**, never run.
- Actual system XKB keymaps: German, French, Spanish, Turkish, Polish, Dvorak,
  Hungarian and Japanese kana-off fixtures passed; Shift/AltGr, reverse lookup,
  dead-key/control fallback and overflow were exercised. This is offline
  libxkbcommon testing, not native capture/injection testing.
- Pure tests include hostile text/frames, exhaustive Unicode control/format
  rejection, a deterministic 100,000-transition randomized held-key/layout
  invariant check, bounded malformed resolver output, rate limiting, old wire
  compatibility and preserved physical modifiers. These are deterministic
  randomized tests, not coverage-guided fuzzing or real keyboard testing.
- Real debug CLI/null daemon: help, invalid mode, missing peer, peer override,
  existing-settings upgrade/preservation, malformed-settings refusal, reload,
  status/doctor fallback, normal restart and SIGKILL/stale-socket restart passed.
- Advanced screenshots for all three saved preferences were visually inspected
  at 980x1200. Native Windows/macOS rendering and other window sizes were not
  validated.
- Null-daemon idle samples: requested characters, effective physical: zero CPU
  ticks over two seconds, RSS 31,052 KiB; requested/effective physical: zero ticks
  over two seconds, RSS 31,116 KiB. These short samples do not establish zero
  idle CPU generally or measure a functioning character-mode path.

## Owner validation script (required before enabling TEXT)

This is a future acceptance script, **not** a claim these paths work now. On
the current build the characters/auto commands must explicitly report physical
fallback. Use dedicated test computers and a text editor, never a shell prompt,
for native typing tests. On each machine save its existing settings first.

```sh
one-mk typing
one-mk typing --keys characters --peer TEST_PEER
one-mk status
one-mk doctor
```

1. Start two test machines with old settings; confirm physical behavior is
   unchanged. Pair old and new builds and check capability fallback.
2. German sender, US receiver: enable characters and type z/Y, Shift levels,
   AltGr+Q (@), AltGr+E, dead-accent then vowel, repeat, Ctrl+Z/Y and copy/paste.
3. French and Polish real keyboards: AltGr symbols/letters, then Spanish,
   Turkish dotless i, Hungarian AltGr+B, Dvorak. Test Caps Lock and group changes.
4. Repeat on Windows and macOS in both directions. These OS paths are NOT
   runtime-verified here. Test permission revoke, reconnect and layout change
   while shortcut keys are held; no key or modifier may remain down.
5. Private Xvfb only, clean environment with no D-Bus/Wayland variables: change
   layouts, type into an X client, kill the injector with SIGKILL during remap,
   restart and compare the entire original keymap. Never use the live display.
6. Exercise the Advanced preference UI and inspect screenshots. Measure idle
   CPU/RSS with typing off/on. Fuzz framing, text and visit transitions.
7. Restore physical mode on each sender with
   `one-mk typing --keys physical --peer TEST_PEER` and
   `one-mk typing --keys physical`. Peer overrides take precedence over global
   settings; restore the test machines' original settings afterward.

Until those paths exist and pass, 5E is not complete and Same letters must not
be presented as an operational or recommended feature.
