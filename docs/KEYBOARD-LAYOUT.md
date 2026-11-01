# Keyboard layouts and AltGr (Wave 5E)

## Status and safety boundary — November 2, 2026

**Character typing ships in ordinary release builds on macOS and Windows
receivers.** Keys remains the default, preserving existing shortcuts, raw-input
applications and old-peer compatibility. The window's Keys, Letters and Auto
choices are enabled. Preferences apply to outgoing typing.

| Platform | Sender | Receiver |
| --- | --- | --- |
| macOS | Current TIS keyboard layout and UCKeyTranslate, with visit-owned dead-key state and Option levels | CGEventKeyboardSetUnicodeString on keyboard down/up events with the existing source/tag |
| Windows 10 1607+ | Foreground thread's HKL and ToUnicodeEx flag 0x4, with visit-owned accent composition and AltGr | SendInput KEYEVENTF_UNICODE down/up pairs, including UTF-16 surrogate pairs and the existing tag |
| Linux X11 | Read-only live XKB snapshot and xkbcommon compose | **Physical keys only: received letters follow this computer's own layout** |
| Linux Wayland portal/libei | Captured keyboard's supplied XKB map and modifier/group events, with xkbcommon compose | **Physical keys only**; NotifyKeyboardKeysym stays unadvertised |
| Linux wlroots | Seat XKB map and modifier/group events, with xkbcommon compose | **Physical keys only** |
| Null | Explicit debug fixture layouts only | Fixtures record Text; ordinary/release null does not advertise text |

macOS and Windows are **compile-checked only here**, not hardware-validated. The
checklists below remain owner validation, not a completed application matrix.
The optional native-local-remap experimental backends are separate and unchanged;
this implementation targets the ordinary native backends used by releases.
Windows before build 14393 falls back physically rather than use an unsupported
non-mutating flag. No clipboard transport, X11 keymap remapping, polling loop or
idle timer is introduced; native held letters reuse existing receiver repeat.

## Current preferences, protocol and cleanup

- **Keys** (`physical`): physical evdev-position keys, always.
- **Letters** (`characters`): characters whenever sender translation and receiver
  TEXT support are available, including matching layouts.
- **Auto**: characters only with valid, different descriptors and receiver TEXT;
  equal/missing descriptors or unsupported translation means physical keys.

`one-mk typing --keys physical|characters|auto [--peer ID]` saves a global or
full-fingerprint peer preference. Preference changes take effect next visit.
Source layout changes are read on keyboard activity or supplied by the Wayland
seat; Auto re-evaluates descriptors without changing a held key's release.
Receiver descriptors refresh at entry, without a background watcher.

Native descriptors include their operating-system layout family: German on
macOS and German on Windows are not treated as equal because their Option and
AltGr levels differ. Matching names alone do not force Auto to physical keys.

Protocol 2 retains its existing Text, CharacterKey, KeyboardLayout and physical
Key frames. TEXT (64) means receiver support; TEXT_SOURCE (1024) means sender
translation without promising text reception; TEXT_HOLD (2048) means the receiver
can hold/repeat native CharacterKey Unicode until its up. No text goes to a peer
without TEXT. Unknown bits are ignored by old peers. A TEXT receiver without
TEXT_HOLD gets bounded Text events, retaining the older reverse-chord receiver
for legacy CharacterKey messages.

Emergency/local hotkeys precede mapping, then translation, then Ctrl/Cmd swap.
Ctrl, Cmd/Super and ordinary left Alt shortcuts stay physical. Right Alt selects
AltGr; on macOS Option selects text unless Ctrl/Cmd is also held. Windows capture
filters AltGr's synthetic Ctrl (scan 0x21d); real Ctrl is never guessed to be
AltGr. No text is emitted with a shortcut modifier held, including repeats after
Ctrl/Cmd is added. Shift/AltGr/Option are lifted/restored around Unicode commits
in reliable-stream order, and locally around native text repeats.

Every down retains its resolution until up. Pending dead-key downs/ups are
suppressed; shortcuts, layout changes and visit end reset composition. Text is
printable, one grapheme, at most eight UTF-8 bytes; resolved sequences are bounded
to 32 bytes and split into validated graphemes. Source-key spoofing, controls,
format characters, mismatched releases and non-owner text are refused. Text and
character downs share a 200-per-second budget; matching ups are never rate-limited,
including after source capability loss. Disconnect, revoke, timeout and visit end
release all remembered keys and clear native text/repeat ownership.

Unsupported layout data falls back physically. IME candidate/pre-edit state and
arbitrary custom Windows dead-key tables are not qualified. Windows composition
handles standard Unicode accents, dead+space, repeated accents and noncombining
fallback, not a custom DLL's arbitrary dead-key table. Multi-scalar or multi-
grapheme commits have no native held-character representation: their repeat
follows source repeat events, which libei does not guarantee.

## X11 and Wayland receiver decisions

The X11 rejection stands: immediate remapping failed **50/50** strict runs.
Four retained/reassigned slots produced only **1,432 correct symbols out of
10,000**, despite complete map/down-state restoration. A finite pool or local
generation counter is not destination-consumption acknowledgement. **No X11
keymap-remapping receiver was retried or shipped.** See
[the original evidence](reviews/WAVE6-X11-TYPING-REPORT.md).

The existing fake-tested NotifyKeyboardKeysym helper stays unadvertised.
RemoteDesktop forbids Notify after ConnectToEIS on the same session, so it uses a
second keyboard session. Ordering between libei modifiers and Notify text, real
GNOME/KDE application delivery and session-loss behavior remain unqualified;
fakes cannot close those gaps. Physical reception is the safe fallback.

## Current verification

Targeted tests cover US, German, French AZERTY, US-International and Arabic
fixtures; real xkbcommon tables/compose; shortcuts; dead sequences; UTF-16 BMP,
combining and surrogate pairing; modifier release/restore; repeat suppression;
visit cleanup; hostile input and old-peer negotiation. Composition belongs to
each visit: a three-daemon broadcast regression commits apostrophe+e as **é**
independently on both receivers, and supplied-XKB tests exercise the same
isolation without fixture translation.

The two-process debug-null test starts authenticated daemons with German → US
Auto, records **äöü@€ ß** as ordered Text, holds physical Ctrl+key21 (German Z),
SIGKILLs the controller and checks both releases with no shortcut Text. This is
real daemon/control-path evidence, not native desktop evidence. Input/protocol
crates compile for both requested native targets. Full daemon checks additionally
need cross C toolchains for ring, not just installed Rust targets.

Final validation on this Linux host:

- `cargo clippy --workspace --all-targets -j 2 -- -D warnings` passed. A final
  input/protocol/daemon clippy pass also passed after the visit-isolation change.
- The single `cargo test --workspace -j 2` invocation found two stale
  shortcut/repeat assertions. Those assertions and the obsolete capture-source
  snapshots were updated. Final partitioned runs (`-p one-mk -p one-mk-input`
  and `--workspace --exclude one-mk --exclude one-mk-input`, both `-j 2`)
  passed every workspace package and doctest: daemon units **420 passed,
  5 ignored**; input units **278 passed, 6 ignored**, plus their integration
  suites and the remaining crates. The original whole-workspace invocation
  itself did not pass; it was not rerun.
- Both requested native Rust targets are installed. Input and protocol checks
  passed for `x86_64-apple-darwin` and `x86_64-pc-windows-msvc`. The requested
  checks including the daemon could not complete: ring's C build used a Linux
  compiler that rejects Apple's flags, and the Windows build lacks `lib.exe`.
  No macOS or Windows executable was linked or run here.

Validation used `CARGO_TARGET_DIR=/home/samer/.t3/cargo-target/typing`, `-j 2`,
no formatting command and no fat LTO. Logs are retained in `/tmp/one-mk-typing-*`;
the dedicated build directory is removed at handoff.

## Manual Mac checklist (ordinary release build)

1. Grant Accessibility/Input Monitoring. Pair a supported receiver, open a text
   editor there, set German controller / US receiver and verify Keys first.
   Choose Letters, re-enter, and type **äöü@€ ß**, Shift levels and Option text.
2. On US, try Option+e then e (é), dead+space and cancelled sequences. Switch the
   controller's current source through US, French AZERTY and Arabic; each new
   press must follow it. Check Auto with equal/different layouts and Letters
   with matching layouts.
3. Hold/release letters, Shift and Option in both orders; verify repeat stops.
   Add Cmd/Ctrl while held. Try Cmd+Z/C/V, Ctrl shortcuts, Enter/Tab/Escape and
   arrows: shortcuts must not insert text.
4. Hold keys/modifiers while switching home, revoking, SIGKILLing either daemon
   and reconnecting. Nothing may stay stuck/repeating. Repeat with the Mac as
   Unicode receiver; include a non-BMP character and a combining grapheme.
5. Verify older peers and Linux receivers remain physical. Record OS/editor/
   layout combinations and failures; Unicode-ignoring/secure apps may need Keys.

## Manual Windows checklist (Windows 10 1607+ / Windows 11)

1. Run ordinary releases at equal integrity levels. In Notepad on a US receiver,
   choose Letters and type **äöü@€ ß** from a German Mac/Windows sender. AltGr+Q/E
   must produce @/€ without menus or fake-Ctrl shortcuts.
2. Change the foreground input language through US, French AZERTY and Arabic.
   On US-International test apostrophe+e, quotation+u, dead+space, repeated accents
   and noncombining sequences. Returning home must preserve local dead-key state.
3. Check Keys, Letters, and Auto with matching/different layouts. Test Ctrl+Z/C/V,
   real Ctrl+Alt, Win shortcuts, Alt+Tab, Shift and Enter/Tab/Escape; no shortcut
   text commits. Receive 😀 and a combining grapheme without replacement glyphs.
4. Hold/release letters; add Ctrl while held; switch layouts/visits, revoke and
   SIGKILL the controller. Verify repeat and all modifiers stop, in both directions.
5. Test UAC/admin/lock-screen boundaries separately: Windows can block SendInput
   and should report its existing input error, not claim delivery. Restore Keys
   and record actual native results before qualifying the application matrix.

## Historical qualification notes (October 30–31, 2026)

The dated sections below preserve earlier investigations and commands. Their
statements that *all* native TEXT is disabled, capture wiring is absent, or
Windows/macOS are still future work are **superseded by the current status above**.
Their X11 delivery counterexamples and Wayland qualification gaps still apply.

## 5E-5 verified partial delivery (October 30, 2026)

**Not a completed AltGr fix or enterprise runtime sign-off. Production TEXT
advertisement stays off for every native backend.** The requested condition
that both native injection paths pass was not met, so capture snapshot/descriptor
wiring and the two-real-daemon native layout matrix were deliberately not added.

### Portal: protocol implementation, not desktop qualification

The Linux portal injection worker now routes bounded printable `Inject::Text`
through `NotifyKeyboardKeysym` press/release pairs. Latin-1 uses the legacy
keysym value; other Unicode scalars use `0x01000000 | scalar`. Direct
`Inject::CharacterKey` uses the same path with source-code/character pairing,
repeat handling, alias ownership, mismatched-release refusal and physical
key-up cleanup. No keymap is rewritten and no clipboard is involved. Invalid
character/source combinations are rejected before opening a permission dialog;
Text while a physical Ctrl, shortcut Alt or Super is held is refused.

The helper requires RemoteDesktop version >= 1, the advertised keyboard device
bit, the exact introspected `NotifyKeyboardKeysym(o, a{sv}, i, u)` method, and
the keyboard bit actually granted by `Start`. These are necessary protocol
checks, **not sufficient grounds for capability advertisement**. There is no
verified portal-keymap reverse lookup/unambiguous physical-code shortcut yet.

The upstream RemoteDesktop interface forbids `Notify*` after `ConnectToEIS`
on the same session. Consequently the existing libei physical/pointer session
is preserved, and the helper creates a separate, keyboard-only Notify session
lazily on the same dedicated injection connection. This can require another
permission dialog; it does not save another restore token. Physical-only visits
create no extra session, poller or periodic timer. Separate sessions do **not**
prove ordering between libei modifiers and Notify symbols.

The fake portal rejects Notify on EIS sessions and records exact symbol/state
pairs. Tests verify Unicode, combining scalars, 200 ordered mixed characters,
pairing/repeat/aliases, hostile input refusal, visit end, shutdown, lock/sleep,
symbolic session Closed, failed press/release compensation, failed visit
cleanup, cancellation of an unacknowledged press on lock, and cancellation of
a pending permission dialog on physical-session loss. Visit end, injection
channel disconnect and physical key-up also cancel pending permission;
visit end/disconnect interrupt an unacknowledged press. Already-ended queued
visits do not open a keyboard dialog. New input buffered while awaiting the
portal is bounded to 256 queued messages before failing closed. Physical
key-up during an unacknowledged symbol send fails closed rather than delaying
modifier cleanup. Cleanup preserves held
state after failed releases until the session is closed; it is bounded by the
existing call timeout. Main/symbolic Closed, portal owner loss and power changes
interrupt symbol sends. On panic or a lost bus the dedicated connection is
dropped; release then depends on the compositor's session/client teardown.
Actual compositor teardown on bus/service loss is **not** established by these
fake tests; existing physical bus/owner-loss regressions are not symbolic
hardware acceptance.

One clean fake-portal test run measured Text press+release acknowledgement
P50 **748 microseconds**, P95 **1,019 microseconds** over 200 scalars. This is
local fake D-Bus acknowledgement only, not editor delivery latency, native
character-mode overhead or a network/desktop benchmark. Native character-mode
P50/P95 and on/off idle CPU ticks were **not measured**. No zero-tick claim is
made for this change.

The daemon receiver still resolves CharacterKey through its existing reverse
lookup/physical-key route, and native backends still expose no keyboard
descriptor for TEXT negotiation. Thus passing backend tests is **not** a
two-daemon portal text/shortcut result. Real GNOME and KDE are **UNTESTED**.

### X11: finite retention still fails delivery acceptance

The new private-Xvfb experiment reserves four unused, unheld, non-modifier
keycodes, retains one mapping for each of `ı`, `ł`, `@`, `z`, and reuses each
slot for a burst of 200 characters. Both an independent XKB reader and a real
SIGSTOP-delayed xev/Xlib client translate the burst. The real xev check verifies
ordered presses and matching following releases, ignoring surplus XTest
pre-press releases; the independent reader verifies exactly 200 presses.
The xev assertions inspect keysyms, not XIM commit strings or editor output.

| Mapping lifetime | Characters resolved, in order | Full core/XKB maps restored | Other client's held key preserved |
| --- | --- | --- | --- |
| Retained until both test readers translate | 200/200 | yes | yes |
| Restored after 1.6 seconds, before readers resume | 0/200 | yes | yes |

`XSync` and minimum mapping age do not acknowledge destination application
translation. A busy/stopped client may outlive any finite quiet/LRU deadline.
This is a counterexample to the proposed finite-retention safety claim, not
proof that a longer timeout fixes it. Keeping mappings until these *known test
readers* finish does not provide a production acknowledgement mechanism for
arbitrary GTK/Qt/Xlib clients. It also does not establish all 200 distinct
characters: the burst deliberately reuses four slots.

Reproduce only on private X servers (the script supplies a clean environment):

```sh
bash scripts/test-x11-typing-safety.sh --pool-retention
bash scripts/test-x11-typing-safety.sh --pool-retention --require-delivery
```

The first command passes by asserting both results. The second **fails as
expected**, after reporting retained delivery 200/200 and retired delivery
0/200. The original single/32 immediate-restore regression remains unchanged
in meaning and passes by reproducing zero delivered characters.

No production pool, quiet timer, LRU eviction or root-property recovery journal
is shipped. Therefore SIGKILL-during/after-remap restart recovery is **missing**,
not verified by unwinding test guards. No arbitrary other-application keymap
compatibility or delivery guarantee is claimed. A journal alone would recover
global state but would not repair already-lost characters. Native TEXT remains
off rather than masking the acceptance failure with timing assumptions.

### Owner-only GNOME/KDE manual probe

Never run this against the owner's desktop during automated QA. The owner may
run these explicitly from their GNOME/KDE Wayland terminal, choosing the named
layout manually first and using a disposable editor with no valuable content:

```sh
bash scripts/test-portal-typing-manual.sh --owner-desktop de
bash scripts/test-portal-typing-manual.sh --owner-desktop fr
bash scripts/test-portal-typing-manual.sh --owner-desktop pl
```

Turkish is also available as `tr`. The script refuses without the explicit flag
and a Wayland session bus, builds the opt-in `portal-typing-probe` example in a
clean environment without a desktop bus, then passes only the explicitly
authorized bus into the isolated probe. It never changes layouts/config,
starts a daemon or accesses the clipboard. Approve the keyboard portal dialog,
then focus the disposable editor within five seconds. Compare the displayed
expected string exactly, including case, spaces, Euro/AltGr symbols and Polish
letters. An acknowledged method call or successful exit is **not** a pass if
the editor differs. The script cleans up its own child and temporary state.

This probe tests **Text only**, not capture, CharacterKey shortcuts, repeats,
layout switching or disconnect cleanup through a real daemon. Record compositor
and portal versions, layout/variant, editor, actual text and permission-dialog
behavior. Do not enable TEXT based on this probe alone. Before native sign-off,
complete German Z/Y and AltGr+Q, French AZERTY/AltGr, Polish letters, Turkish
dotless i, Ctrl+Z, repeat, layout-switch-mid-hold, peer SIGKILL, old-peer
fallback, unchanged full keymaps, native P50/P95 and zero-idle-tick checks from
the acceptance matrix. Those results are **absent** in 5E-5.

Hostile self-review: a fake portal proves protocol ordering, not compositor
layout semantics, editor composition or grapheme behavior. Two sessions may
break shortcut modifier ordering. A direct Unicode keysym need not be handled
identically by GNOME and KDE. Finite X11 retention remains falsified even if
ordinary apps happen to consume quickly. No snapshot or production-capability
wiring is justified by the present evidence.

Validation for this slice: clean-environment `cargo test --offline -j 2
--workspace` passed (1,054 passed, zero failed, 14 ignored). The portal-only
selection passed 34 tests. Strict workspace/all-target Clippy, input
all-features/all-target Clippy, and the requested input/proto/ctl/app/tray
all-target cross-Clippy checks for `x86_64-pc-windows-msvc` and
`aarch64-apple-darwin` passed with `-D warnings`. Cross checks are compile-only.
The retained-pool experiment passed its positive/counterexample assertions;
strict `--require-delivery` failed specifically on retired delivery 0/200.
Shell syntax and manual-script refusal paths passed. A final process audit
found no remaining private Xvfb/xev/dbus-daemon processes with this run's test
environment. Runtime tests used private fake portals/Xvfb only, with throwaway
HOME/runtime/state; the owner's desktop, clipboard, settings and daemon were
not used. No cargo fmt was run.

## 5E-4 release blocker: restored mapping is not delivered text

The October 30, 2026 failing-first X11 experiment **did not implement 5E-4**.
It found a correctness blocker in immediate temporary-remap/XTest/restore
injection. At that slice, production capability advertisement and injection
were unchanged. X11, portal/GNOME, wlroots, Windows and macOS still use physical fallback.
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
Native backends normally treat CharacterKey as physical and refuse Text;
the unadvertised portal helper described above is the experimental exception.
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
  clients and restoration on every error and at startup, **plus a delivery-safe
  lifetime**. Neither XSync nor finite quiet/LRU ages establish client
  translation. RAII alone cannot survive SIGKILL. Do not ship a remapping
  injector until both delayed-reader delivery and SIGKILL recovery pass on a
  private X server. Never alter the owner's live map.
- Portal: the unadvertised NotifyKeyboardKeysym helper exists; qualify real
  GNOME/KDE delivery, cross-session modifiers, keymap reverse lookup and
  session-loss cleanup before enabling native negotiation. Wlroots still needs
  a safely regenerated virtual keymap or explicit physical fallback.
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
