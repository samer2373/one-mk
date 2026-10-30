# Keyboard layouts and AltGr (Wave 5E)

## Status and safety boundary

This is an **incomplete native implementation**, not a claim of desktop
cross-layout support. The default remains physical keys. Slice 5E-3 wires
descriptor exchange, per-visit negotiation, sender routing, receiver validation,
reverse lookup and held-key cleanup through the real daemon control path.
Debug test null backends provide per-instance fixture layouts and record Text.
Native snapshots and compose are **not connected to capture events or visits**;
native text injection remains missing. Native and normal null backends therefore
send physical keys, even when characters or auto is requested, and never
advertise TEXT. Test support requires both `null-typing-tests` and debug
assertions; release builds cannot construct fixture keyboards, even with that
feature. No polling or background timer is introduced by character routing.

## 5E-4 release blocker: restored mapping is not delivered text

The October 30, 2026 failing-first X11 experiment **did not implement 5E-4**.
It found a correctness blocker in immediate temporary-remap/XTest/restore
injection. Production capability advertisement and injection remain unchanged;
X11, portal/GNOME, wlroots, Windows and macOS still use physical fallback.
The native sender snapshot is still not wired to the daemon. Do not describe
this experiment as working X11 character mode.

The regression uses independent injector and reader clients on a private Xvfb,
plus the real Xlib-based `xev` client. It selects an
all-NoSymbol, unheld keycode outside the modifier map, maps every level to a
Unicode keysym, verifies that a native XKB snapshot resolves the intended
character, sends XTest press/release requests with server round trips, and restores the mapping.
Only afterward does the destination client process its queued MappingNotify
and KeyPress events and obtain its current XKB snapshot. It receives every key
but resolves **no text**: the keycode is already unmapped again. This models a
busy application whose event loop runs after the injector has completed.
The test also suspends its own xev process with SIGSTOP (waiting for stopped
status), then resumes it only after all mappings have been restored. xev reports
NoSymbol for every injected press and release, independently confirming the
result with Xlib, not just the small XKB reader.

Observed results, for this experiment only:

| Remaps | XTest presses received | Core map restored | Full `xkbcomp` dump restored | Other client's held key preserved | Characters resolved |
| --- | --- | --- | --- | --- | --- |
| 1 (Turkish dotless i) | 1 | yes | yes | yes | 0 |
| 32 (dotless i, Polish ł, @, z repeated) | 32 | yes | yes | yes | 0 |

Server round trips prove that X has processed requests, not that an application
has translated the queued key event. MappingNotify is not a historical keymap
snapshot. A recovery property, panic guard or startup repair can address leftover
mapping state, but none provides acknowledgement from the destination
application. Adding a fixed sleep would only hide the race for sufficiently fast
clients; it is not a delivery guarantee. A design that keeps mappings longer
needs a separate bounded-lifetime, ownership, concurrent-layout-change and
crash-recovery argument before it can qualify for TEXT.

Reproduce without touching the desktop:

```sh
# Passing safety regression: confirms the remap delivery counterexample.
bash scripts/test-x11-typing-safety.sh
# Original acceptance assertion: deliberately fails with no text vs U+0131.
bash scripts/test-x11-typing-safety.sh --require-delivery
# The same clean/private-display runner can wrap the normal checks.
bash scripts/test-x11-typing-safety.sh cargo test -j 2 --workspace
```

The runner requires Xvfb, xkbcomp, xev and stdbuf, supplies throwaway HOME, XDG_RUNTIME_DIR
(0700) and ONE_MK_HOME, clears the environment (no Wayland or session D-Bus),
and stops its X server on exit. Cargo's existing registry/toolchain directories
are retained explicitly; build output stays in the checkout, not the RAM disk.
The regression is ignored in ordinary cargo runs and must be selected explicitly;
the runner's default invokes that exact test, with `--ignored` and `--nocapture`.
All remapping, focus changes and held-key probes occur on the regression's own
additional Xvfb, never the display of a running desktop or daemon.

Not verified by this experiment: two real character-mode daemons, capture-side
layout event wiring, shortcut reverse lookup through native injection, compose,
repeats through a destination application, a layout switch during injection or
a hold, SIGKILL recovery, old-peer native fallback, character-mode P50/P95
latency, or on/off idle CPU ticks. There is no operational character-mode path
to benchmark. Restoring byte-identical maps in this test is explicitly **not**
evidence of crash safety or correct delivery to GTK/Qt applications.

Hostile self-review: the observer deliberately refreshes from the current map;
it is a small x11rb/XKB reader. The independent xev/Xlib counterexample is also
verified, but GTK/Qt applications are not. Delayed-consumption scheduling is
deterministic, using SIGSTOP/waitpid/SIGCONT rather than a timing guess; the
startup readiness helper alone waits in short bounded intervals. The test-only restoration guard
is best effort on unwinding, not a production remapper and not SIGKILL-safe.
No changes to production paths were made to turn this counterexample green.
5E-4 remains blocked pending a delivery-safe design and the full native
acceptance matrix below; crash recovery alone is not sufficient.

Checks for this regression-only change: clean/private-display workspace tests
passed (1,005 passed, zero failed, 13 ignored); the separately selected regression
passed and `--require-delivery` failed as expected. Strict workspace/all-targets
Clippy and the requested input/proto/ctl/app/tray all-targets cross-Clippy checks
for x86_64-pc-windows-msvc and aarch64-apple-darwin passed. No cargo fmt was run.

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
authenticated controlling visit, subject to its supported backend, peer TEXT
capability and rate limit. Only the debug null test path currently supports it.
Preference changes take effect on the next visit, never mid-hold. Live status,
doctor and the window report the effective mode, including the source and target
layout (for example `characters (German to US)`).

## Additive protocol-2 wire format

All fields are little endian, inside the existing u16-length-prefixed control
frame. Existing `Key` tag 5 is unchanged. TEXT is capability bit 64, deliberately
absent from LOCAL. A capable backend adds it per connection; only the debug
null fixture backend currently qualifies. Descriptors are sent only after the
other peer advertises TEXT; missing capability always preserves physical keys.

| Tag | Fields | Rules |
| --- | --- | --- |
| 21 Text | u16 byte length, UTF-8 bytes | 1..8 bytes, exactly one extended grapheme cluster |
| 22 CharacterKey | u32 evdev code, u8 down (0/1), u32 Unicode scalar | printable main-block source code only; unmodified printable single scalar; repeat down uses original resolution |
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
bursts close together. The enabled daemon receive path enforces this budget for
Text and CharacterKey downs/repeats together. Matching releases are never rate
limited. Physical/character source-kind mismatches and changed-character ups
are refused; CharacterKey cannot spoof Enter, Tab, Escape or modifiers.
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
tests this; the null fixture daemon visits exercise the integrated path. The normal
daemon key path now uses the receiver's same reference accounting via
`Receiver::physical_key`, rather than keeping physical Shift/AltGr ownership
separate from resolved chords. The daemon's `HeldKeys` mirrors actual injected
keys/buttons for visit-end cleanup. Engine and daemon-adapter tests interleave
forwarded Shift with a resolved shifted chord in both orders; neither releases
the other's reference. Text reaches `Inject::Text` only on capable backends.
`Inject::CharacterKey` is an explicit primitive recorded by null; the daemon
normally resolves it into accounted `Inject::Key` events before injection.
Native backends explicitly treat CharacterKey as physical and refuse Text.
Wire Text has no physical source code, so native physical fallback must happen
at sender negotiation, not by guessing a key at the receiver.
Source codes are bounded to the evdev range so the pure engine can remember at
most 767 source keys. Reverse-lookup chords accept only Shift/AltGr modifiers;
invalid resolver output falls back physically rather than injecting arbitrary
modifiers. The existing per-device mapper must merge its output before this
computer-wide typing state machine; it is not a replacement for device pairing.

One layout per computer is the baseline. Device IDs remain attached through
mapping, but per-keyboard layouts exist only when the sender OS exposes them.
Japanese kana/IME composition and remote IME state are out of scope; use
physical mode with a matching receiver layout.

## Native X11 snapshot and compose APIs (5E-2 partial)

`one_mk_input::X11Keyboard::connect(display)` requires an explicit nonempty
display. It dlopens libxcb and libxkbcommon-x11 in addition to the existing
libxkbcommon loader; there is no new link-time dependency. `snapshot()` obtains
the actual core-device keymap and depressed, latched, locked and effective-group
state. It does not infer the map from `_XKB_RULES_NAMES` or invoke a command.
A short server grab prevents a concurrent layout change between keymap and
state queries; its guard ungrabs on return/unwind and X11 releases it when the
connection dies. Snapshots own their keymap, so an older snapshot remains usable
after the server's map changes. No keyboard map is rewritten by this API.

This is an on-demand snapshot primitive, **not** an event-driven capture cache:
each call reads the map again, allocates XKB state and briefly grabs the server.
It is deliberately not called in the daemon's key hot path yet. Before enabling
characters, wire XKB map/state notifications and use a coherent cached snapshot
after key mapping, without subscribing an idle polling timer. Handle reconnect,
device replacement and failed queries as physical fallback. The snapshot API
has no descriptor exchange or custom-map equivalence claim.

Reverse lookup now searches non-keypad keys at all supported Shift/AltGr levels
before considering keypad keys. Thus US `*` is Shift+8 and `+` is Shift+=, not
KP_Multiply/KP_Add. Regressions cover `* + / - .` and every digit on US, German
and French maps. This preserves shortcut key identity as well as the character.

`LayoutComposer::new(locale)` separately loads the optional compose symbols and
tables. Its explicit Pending/Text/Cancelled/Physical results never return a
control/format character or an oversized/multi-cluster Text. The caller must
buffer pending physical presses and releases and own whole-sequence fallback;
this buffering is **not implemented in the daemon**. Reset composition on visit
end, map change and cancellation. If symbols/tables are absent, replay/fall back
physically for the entire sequence. Do not mix an already forwarded dead key
with later composed Text. IMEs are not supported by this compose primitive.

## Remaining native implementation requirements

- Linux: connect the snapshot/compose APIs to capture and visit routing. Null
  fixture layouts and backend capability gating are wired. X11 must subscribe
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

## 5E-3 daemon verification (October 30, 2026)

The enabled path uses debug-only null fixture keyboards for US, German, French,
Polish and Turkish layouts, including Shift and AltGr levels. These tables are
deterministic test data, not a native layout implementation: lock state, IMEs,
dead keys and custom variants remain outside this slice.

- Paired daemon integration covers German Z/Y, AltGr+Q, French AZERTY/AltGr
  symbols, Polish AltGr letters, Turkish dotless i, Ctrl+Z, repeats, mapper-before-
  resolution, Ctrl/Cmd swap, layout changes mid-hold and physical emergency exit.
- Three daemon tasks broadcast from German to US and French receivers; each
  resolves the same shortcut in its own layout. Missing TEXT and equal-layout
  auto visits retain physical routing. Compatibility uses a simulated old peer,
  not a separately built historical executable.
- Two independent OS processes run the real daemon on null with German/US
  fixtures. Negotiation, Text, Ctrl+Z and SIGKILL of the sender verify receiver
  release of both the resolved key and Ctrl. Separate disconnect and visit-end
  tests cover cleanup. No native editor or native remapping is involved.
- Enabled hostile-peer tests exercise ownership/link consent, capability,
  invalid/control/format/bidi/oversized Text, printable source-code restrictions,
  rate limits, pairing spoofing and releases after exhaustion. A deterministic
  20,000-transition generated sequence checks the held-key invariant.
- Six temporary mutations (routing disabled, capability/consent/source-code/
  budget/pairing guards removed) were each caught by their regression test;
  all source mutations were restored before the final checks.
- Two connected enabled idle processes, after two seconds settling, measured
  **zero process CPU ticks each over three seconds**. This is a bounded Linux
  idle sample, not a universal CPU or native-injector performance guarantee.
  A loaded workspace rerun observed one receiver tick; a subsequent isolated
  rerun observed one tick on each process. Existing QUIC keepalives remain
  active. Absolute zero connected-idle CPU is therefore **not established**.
  CPU sampling is a separate, explicitly ignored manual measurement test
  (`enabled_connected_idle_sample`), not a timing-sensitive zero-tick assertion
  in the always-enabled two-process lifecycle test. No transport timer was
  added or changed by this slice; see [existing idle budgets](IDLE-WAKEUPS.md).

All processes use throwaway HOME/runtime/settings with no desktop or session
bus inherited. Native X11 suites use private Xvfb only. Native text injection,
crash-safe remapping and native capture integration remain unverified and
unshipped. Historical verification sections below describe earlier slices,
not the current null fixture path.

Final clean-environment validation: `cargo test -j 2 --workspace` passed
972 tests, zero failed, nine ignored (including the child fixture and manual
CPU sampler). `cargo clippy -j 2 --workspace --all-targets -- -D warnings`
passed. Both cross-clippy commands passed with `--all-targets -- -D warnings`,
targets `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`, packages
`one-mk-input`, `one-mk-proto`, `one-mk-ctl`, `one-mk-app`, `one-mk-tray`.
These are compile checks, not Windows/macOS runtime validation. The final
separate `cargo test -j 2 -p one-mk enabled_connected_idle_sample -- --ignored
--nocapture` again observed `(0, 0)` ticks; the nonzero samples above still
preclude an absolute-zero claim. No `cargo fmt` was run. Window text is wired
to live status but was not visually inspected in this slice.

## Historical 5E-2 follow-up verification (October 30, 2026)

This follow-up is a **verified partial deliverable, not working character mode**.
The worktree was reset to local wave5 at `3389e12` before implementation.
Tests and daemon runs used clean environments, private Xvfb servers, throwaway
HOME/runtime/ONE_MK_HOME, no Wayland/D-Bus session variables, no real clipboard
and at most two Cargo jobs. Servers and daemons started by the checks were stopped.

- Failing-first native snapshot and compose tests failed on their missing APIs.
  The independent review's US `*` regression reproduced keypad code 55 before
  the reverse-lookup fix. Shared physical/resolved modifier tests were added
  before the shared adapter existed.
- Real private Xvfb servers: system `setxkbmap` changes for German Z/Y, German
  AltGr+Q, French AZERTY/AltGr+0, Polish AltGr+L, Turkish dotless i and US Shift+Y.
  XTest drives the server's real modifier state; snapshots follow map changes,
  active us/de group switching and Caps Lock, and retain the old map while Ctrl
  is held across a map change. These are **native query tests**, not sender-to-
  receiver text injection or a real keyboard/editor test.
- System compose tables: German dead acute + e produces é; reset, cancellation,
  Enter/Tab/Escape and invalid-code fallback pass. The original offline layout
  and Unicode validation tests still pass.
- Null-injection, in-process daemon/QUIC integration: a paired hostile peer
  advertises TEXT, sends an over-budget burst, raw control/bidi/format Text and
  CharacterKey messages while holding a normal key. None inject text or release
  the physical key; normal input and visit-end release continue working. This
  verifies **disabled-capability fallback**, not an enabled receiver rate limit.
- Workspace tests passed: 953 passed, zero failed, seven ignored, including doc
  tests. Linux `cargo clippy -j 2 --workspace --all-targets -- -D warnings` passed.
  Both requested cross-target/package clippy checks passed with `-D warnings`:
  `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`. Windows/macOS were
  **compile-checked only**, never run here. No `cargo fmt` was run.
- The real debug null-daemon binary reports physical fallback in status/doctor
  with requested characters. Five-second unpaired idle samples after settling:
  requested physical: zero CPU ticks, RSS 32,848 KiB; requested characters:
  zero ticks, RSS 33,192 KiB. Both were effectively physical, with discovery and
  clipboard disabled. These samples do **not** measure functioning character mode
  or native XKB snapshot activity during typing. ELF dynamic dependencies have
  no link-time xkbcommon/xkbcommon-x11/xcb dependency.

At the end of 5E-2, still missing: native capture-event integration and notification-driven caching;
null fake layouts/text injection; descriptors and per-visit auto negotiation;
sender Text/CharacterKey routing and compose buffering; receiver text injection
and enabled-path rate limiting; durable remap marker/journal, concurrency and
SIGKILL recovery. Consequently the requested two-real-daemon cross-layout editor
matrix, remap SIGKILL test, physical old-peer interoperability run, native typing
latency measurements and enabled-character idle measurements were **not done**.
TEXT remains unadvertised everywhere. No temporary remapping implementation is
shipped without those crash/recovery proofs.

Hostile self-review: normal keyboard events now share the resolved-chord engine,
but actual Text and CharacterKey wire handling is intentionally still disabled.
The adapter tests are not evidence of real native shortcut injection. Native
snapshots do a blocking server round trip and allocate; calling one for every
captured key would need latency validation, not just idle measurements. Compose
Pending is not permission to swallow keys without a complete replay/pairing
policy. Descriptor names alone must never prove custom maps equivalent. Ignored
tests and unsupported backends remain acceptance gaps, not successes.

## Verification of the original foundations (October 30, 2026)

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

This is a future native acceptance script, **not** a claim these paths work now. On
the production build the characters/auto commands must explicitly report physical
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
   Also suspend the destination application until after mapping restoration,
   then resume it and verify the actual text. Map equality and receipt of raw
   keys alone are not acceptance: the 5E-4 regression demonstrates that both can
   succeed while all characters are lost.
6. Exercise the Advanced preference UI and inspect screenshots. Measure idle
   CPU/RSS with typing off/on. Fuzz framing, text and visit transitions.
7. Restore physical mode on each sender with
   `one-mk typing --keys physical --peer TEST_PEER` and
   `one-mk typing --keys physical`. Peer overrides take precedence over global
   settings; restore the test machines' original settings afterward.

Until those paths exist and pass, 5E is not complete and Same letters must not
be presented as an operational or recommended feature.

### Real Windows/macOS keyboard acceptance script

This is a manual future acceptance script for dedicated test machines, **not a
passing test of this build**. Back up each test machine's settings first. On
Windows select the actual German, French or Polish keyboard in Settings >
Time & language > Language & region, then use Win+Space to select it. On macOS
add the actual input source in System Settings > Keyboard > Text Input and
select it in the input menu. Use a US-layout receiver first; repeat with the
machines swapped. Do not assume macOS Option is Windows AltGr: first record
what the same real keyboard/input source types locally in a plain-text editor.

PowerShell, using the test peer's actual paired name/fingerprint:

```powershell
.\one-mk.exe typing --keys characters --peer TEST_PEER
.\one-mk.exe status
.\one-mk.exe doctor
# Perform the editor exercises below, then restore the test preference:
.\one-mk.exe typing --keys physical --peer TEST_PEER
```

macOS Terminal:

```sh
one-mk typing --keys characters --peer TEST_PEER
one-mk status
one-mk doctor
# Perform the editor exercises below, then restore the test preference:
one-mk typing --keys physical --peer TEST_PEER
```

In a plain-text editor (never a terminal prompt), record local output for German
Z/Y, Shift, AltGr+Q and dead acute + e; French A/Z and AltGr symbols; Polish
AltGr+A/L/Z. Then repeat remotely and compare with that local reference. Hold a
letter to repeat, change layouts while it is held, release and type again. Test
Ctrl+Z undo, Ctrl+Plus versus keypad plus, held physical Shift alongside a
shifted shortcut, visit end and peer termination mid-press; no key may stay down.
Check `auto` with equal/different layouts and an old peer. On this patch all
Windows/macOS and normal null/X11 daemon paths still report physical fallback, so a
cross-layout mismatch is expected and must not be reported as a pass. Restore
the original settings/input source after the exercise. Do remap/crash testing
only on private Xvfb servers, not either machine's live desktop.
