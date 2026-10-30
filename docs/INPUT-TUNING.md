# Input tuning and the key mapper (plan 4A)

What LinearMouse does for one Mac, for every computer one-mk sends input
to: change any key, tap/hold keys and layers, pointer speed and
acceleration, scroll direction, speed and mode, modifier-scroll, and mouse
button remapping. Rules apply to everything, to computers of one OS, or to
one computer, so the same mouse can scroll "naturally" on the Mac and
traditionally on the PC.

## Where it runs

Keyboard-layout translation (Wave 5E) is a separate, unfinished feature: see
[KEYBOARD-LAYOUT.md](KEYBOARD-LAYOUT.md). The new typing preference currently
falls back to physical keys on every backend. Its future resolver must consume
the output of `Mapper::key_from`, after physical local hotkeys and before the
Ctrl/Cmd swap; it must retain each press's routing decision through its release.

Everything runs on the **controller** (the computer the mouse and keyboard
are plugged into), in the send path of a visit, before anything reaches
the network. The computer being controlled receives ordinary keys,
buttons, positions and scrolling, so it needs no new version and nothing
about it changes: an older peer gets remapped input like any other. (The
one exception is per-app rules: there the controlled computer says which
app has its focus, which an older one doesn't; see below.)

```
backend Event ─┬─ Key/Button ─► hotkeys (PHYSICAL keys) ─► key mapper ─► Ctrl/Cmd swap ─► Control::Key/Button
               ├─ Motion ─────► pointer tuning ─► RemoteCursor (absolute or relative) ─► Datagram
               └─ Scroll ─────► scroll tuning (reads the mapper's modifiers) ─► Control::Scroll*
```

* **Hotkeys see physical keys.** `[hotkeys]` (Ctrl+Alt+H, ...) and the
  emergency Ctrl+Alt+Shift+Esc match the keys as pressed, before any
  remapping, so a keymap can never make the way home unreachable, and a
  shortcut stays the same whatever the keymap does.
* **The mapper runs before the Ctrl/Cmd swap.** Rules are written in this
  keyboard's terms ("`capslock` becomes `lctrl`"); the swap for a Mac
  target then applies to what the mapper produced, so `capslock -> lctrl`
  becomes Cmd on a Mac with the swap on, just as the left Ctrl key would.
  Outputs that only make sense per OS (`back`, `forward`) are resolved with
  the target OS *and* the swap in mind, so they land right either way.
* **Pointer tuning runs on the raw deltas from the backend**, before
  `RemoteCursor` integrates them into the remote position (absolute mode) or
  the running sum (gaming mode). Deltas stay `f64` end to end, so there is
  no rounding and no drift: the position is the sum of exactly the tuned
  deltas.
* **Scroll tuning** reads the mapper's modifiers (so a key remapped *to*
  Shift drives Shift-scroll too).
* **Only while the pointer is on another computer.** Input at the screen
  edge (held by an edge rule) and local input are untouched.

## Scopes

Every rule has a scope; the most specific one wins, per key (keymap and
buttons) or per field (pointer and scroll):

| scope | TOML | matches |
|---|---|---|
| global | (nothing), `scope = "global"` | every computer |
| OS | `scope = { os = "mac" }` (`mac`, `windows`, `linux`) | targets running that OS (from its `Hello`) |
| one computer | `scope = { peer = "laptop" }` | a paired computer, by name or id (prefix) |
| local keyboard (experimental, default OFF) | `scope = { local = true }` | separate rule set, never a remote target; Windows and partial macOS adapters are compile-checked only, natively UNTESTED; Linux inactive |

| one keyboard or mouse | `scope = { device = "usb/046d:c52b//Logitech USB Receiver" }` | input from that physical device only (see "Per-device rules"); may be combined with `os` or `peer` |

`device` beats `peer` beats `os` beats `global`; a device rule combined
with `peer` or `os` beats the device rule alone. Rules are resolved when a
visit starts, again when `settings.toml` is reloaded, and again when a
keyboard or mouse is plugged in, unplugged, falls asleep or wakes. Any rule
may also carry `app = "..."` (next section): it then applies only while
that app has the focus.

## Per-app rules (plan 4D)

`app = "code"` on a key, button, pointer or scroll rule makes it apply only
while that app has the focus **on the computer the input goes to** (where
the keys land): `Ctrl+Space = F13 only in the editor on the Mac` is a rule
with `scope = { os = "mac" }` and `app = "com.microsoft.VSCode"`.

```
controller (rules)                              controlled computer (focus probe)
visit starts, rules name apps ──WatchFocus{app}──► looks at the focus now,
                                                   then FOCUS_SETTLE after a click, a
                                                   window-switching chord, Enter, Tab, Esc or
                                                   the logo key let go from the controller,
                                                   and when the OS says another window came
                                                   to the front (Windows, X11, macOS; no timer)
re-resolve the rules, ◄──Focus{apps}────────────── only when the names change
Mapper::set_keymap (as a reload)
```

* **Names per OS** (`one_mk_proto::appmatch`, `FocusProbe` in
  `crates/input/src/system*`):

  | target | names reported | how | status |
  |---|---|---|---|
  | Windows | program file (`chrome.exe`) and window class | `GetForegroundWindow`, `QueryFullProcessImageNameW`, `GetClassNameW`; `EVENT_SYSTEM_FOREGROUND` hook says when it changes | compile-checked; unit-tested logic |
  | macOS | bundle id (`com.apple.Terminal`) and app name (`Terminal`) | Accessibility `AXFocusedApplication` → pid → `NSRunningApplication` (`NSWorkspace.frontmostApplication` goes stale in a process without an AppKit run loop); an `AXObserver` for `AXApplicationDeactivated` on the focused app, on a run-loop thread of its own, says when it changes (each probe moves it to the app found) | compile-checked only |
  | Linux, X11 and XWayland windows | both parts of `WM_CLASS` (`code`, `Code`) | `_NET_ACTIVE_WINDOW` on a worker thread; the same thread and connection select `PropertyNotify` on the root window while watched and say when `_NET_ACTIVE_WINDOW` changes (50 ms debounce) | runs on X11; tested against a private Xvfb |
  | Linux, Wayland-native windows | nothing | GNOME and KDE don't tell a background program which Wayland window has the focus (no protocol for it) | unknown: the rules for apps don't apply |

* **Matching**: case-insensitive; `.exe`/`.app` may be left off (`chrome`
  names `chrome.exe`); `*` stands for any run of characters
  (`com.jetbrains.*`); a leading `!` means every app but that one. A
  pattern matches if it names any of the names reported. Patterns with
  control or invisible characters, a folder (`C:\...`, `/usr/bin/...`),
  more than 255 bytes of UTF-8 after any `!` (85 CJK characters, 63
  emoji), or `!*` are refused (the rule is skipped and `keymap check`
  says why); names are cleaned (control and invisible characters
  removed, at most 4 names, each cut to at most 255 bytes at a character
  boundary) by the computer reporting them before they are sent, and
  again on arrival. 255 bytes is what a name may take on the wire, and the
  matcher compares names cut the same way on both sides, so a long
  multibyte name matches (or, for `!`, doesn't) identically here and
  there; the encoder never cuts a name itself.
* **Precedence**: a rule for the focused app beats every rule without an
  app; among rules for the app, `peer` beats `os` beats `global` as usual.
  Pointer and scroll rules for an app override field by field on top.
* **Unknown means off (fail safe).** A peer that can't tell (a Wayland
  window), hasn't said yet (the first moments of a visit), or runs a
  one-mk from before 4D: no rule with `app` applies, `!` rules included.
  `one-mk status` then says, once per computer, `Rule for app X not
  active: <computer> does not report its focused app (why)`.
* **Switching rules keeps the key-state invariant.** A change of app
  re-resolves the rules and hands the new keymap to `Mapper::set_keymap`,
  the same path a reload takes: a held key is released as the code it was
  pressed as, a tap/hold key not decided yet counts as held, layers end
  (releasing what was typed in them); identical rules change nothing. The
  mapper's random-stream property test switches between keymaps and their
  per-app variants mid-stream; `tuning::tests::app_changes_never_leave_a_key_stuck`
  does the same through `TuningSettings::resolve`; a two-daemon test
  (`app::tests::tuning::per_app_rules_follow_the_peers_focused_app`) holds
  a key through a change of app.
* **Wire** (additive, protocol 2): capability `FOCUS_APP` (32).
  `WatchFocus` gets a trailing byte (bit 0: also the app's names, bit 1:
  not the fullscreen state; absent: fullscreen only, as before). `Focus`
  gets a trailing count and names (absent: unknown). Older peers read the
  fields they know and skip the rest; a peer without `FOCUS_APP` is never
  asked for apps.
* **Privacy and cost.** The controller asks only while rules for that
  computer name an app; the names go only to the computer controlling this
  one, only while it asks, and are logged at debug level at most. Watching
  for apps alone sends no fullscreen repeats, and nothing polls: the focus
  is looked at after input that may move it and when the OS says it
  moved. Input that may move it: a click; a chord with Ctrl, Alt or the
  logo key; Alt or the logo key let go; F11, Esc; and, only while rules
  for apps watch that computer, Enter and Tab (launchers, docks and
  switchers change apps with them; not Space or letters, so typing costs
  nothing). The OS says it moved on Windows (`EVENT_SYSTEM_FOREGROUND`),
  for X11 and XWayland windows (`PropertyNotify` on the root window for
  `_NET_ACTIVE_WINDOW`, selected only while watched, on the probe's own
  thread and connection: it never connects to, or starts, XWayland for
  this, and lets go once no X11 window has the focus), and on macOS (an
  Accessibility observer, only while watched). A focus change between two
  Wayland-native windows with no such input stays unseen until the next
  one. Keys typed in the first ~100 ms after a change still use the
  previous app's rules (`app::tests::tuning::per_app_rules_follow_a_switch_made_with_enter`,
  `..._with_no_input`, `system::linux::tests::watch_says_when_the_active_window_changes`).

### Local keyboard scope: experimental native adapters (5H incomplete)

Windows/macOS are EXPERIMENTAL, **COMPILE-CHECKED ONLY, native UNTESTED**. The
acknowledged handoff model and held-key matrix are in `LOCAL-REMAP.md`;
native activation is still incomplete. `[input] local_rules = false` is the
default, including when upgrading old files. `one-mk keymap local on|off`
opts in on native Windows/macOS backends; a healthy acknowledged worker fence
is required before status says `active (N)`. macOS integration is partial:
Caps Lock/Fn/media/function-key sources and outputs reject the entire local
snapshot. Caps Lock is a delayed toggle, not a reliable `flagsChanged` down/up
pair; actual physical Caps Lock remapping remains a 5H-4 blocker. Its original
toggle passes untouched. Mouse buttons and local pointer speed
are outside this feature.

The native-free tests cover the ticketed ownership fence, stale/foreign
acknowledgements, cleanup failure, the held-key routing matrix, randomized
device/transition/failure streams and allocation-free gate decisions. They
do not establish native hook latency or actual injection/release delivery.
The hook-visible gate is now an acquire/release atomic handle; the worker's
ticket state remains private to that worker. Shortcut flags cannot divert an
OS-owned key-up, even during handoff or failure. Native adapters now consume
these primitives; native delivery, modifier flags, no-stuck-keys and idle CPU
acceptance have NOT passed. macOS requires tagged tap receipts after posting;
CGEventPost enqueueing alone cannot acknowledge release delivery.

`one-mk keymap add capslock esc --local` stores a local rule; `one-mk
keymap remove capslock --local` removes it, and `one-mk keymap` labels it
`this computer (local)` and reports backend activity separately. On macOS this
Caps Lock example is stored but deliberately inactive; use a supported key such
as `one-mk keymap add a b --local` in a disposable acceptance session. `--local` cannot be
combined with `--peer` or `--os`, and is not offered for pointer, scroll or
button commands. In the file it is written as:

```toml
[[input.keymap.rule]]
from = "capslock"
to = "esc"
scope = { local = true }
```

The existing remote-only Keys & mouse editor omits local-scoped rules
rather than mislabeling or modifying them as global rules; manage these
local rules through the CLI or file for now.

**Do not treat experimental activation as production acceptance.**
`one-mk keymap check` reports the native UNTESTED limitation. With local rules configured,
Linux `status` adds exactly one availability line:
`local key rules are not available on Linux`. Windows/macOS instead report
`active (N)` or the reason inactive, always labelled native UNTESTED. On macOS,
tap-disable/session/sleep/wake/Secure Input failures request release-all and
pause until all physical keys are released and settings explicitly reloaded.
Accessibility and Input Monitoring permissions are independently required.
See `LOCAL-REMAP.md` for manual acceptance and recovery from another computer.
Remote rules keep their existing
meaning: global/OS/peer rules do not become local rules, and local rules
never enter a peer's mapper. Local compilation targets this computer's OS
without the remote Ctrl/Cmd swap. Invalid and duplicate local rules are
rejected by the local compiler; compiler warnings remain visible in check.

The pure adapter in `crates/input/src/localmap.rs` owns a separate instance
of the same `proto::remap::Mapper`, not a second mapper implementation.
It checks physical emergency/user shortcuts before mapping (including when
Esc or the modifiers are remapped), ignores synthetic keys, preserves press
ownership through repeats and health changes, and releases mapped outputs
on disable, session reset and snapshot replacement. Snapshots are prepared
off the callback; replacement returns the retired allocation for disposal
outside it. Its only mapping deadline exists while a tap/hold decision is
pending. A pure event-driven health lease is provided, but **is not wired
to daemon health**. No worker, idle polling, hook or event tap is added.

Before native interception can be enabled, the bridge must satisfy all of
these safety gates:

* Serialize hook/tap decisions, snapshot changes, timers, injection and
  release-all. A remote capture transition must acknowledge completion of
  every local injection and release before forwarding begins. An unchecked
  background injection queue can replay local keys after handoff and is
  unsafe; it is deliberately not implemented here.
* Hook decisions must not lock or allocate, and must complete within a few
  milliseconds. Wire daemon health acknowledgements and fail-open handling;
  a Windows hook timeout can cause the OS to remove the hook. Never start
  mapping a repeat or release whose down passed through unchanged.
* Windows must reuse the injection tag and ignore `LLKHF_INJECTED`, inject
  scancodes with the right extended flags, preserve AltGr's synthetic
  LCtrl/RAlt handling, and reset/release on session lock or secure-desktop
  transitions. Ctrl+Alt+Del and OS-reserved Windows shortcuts cannot be
  promised as remappable; a normal user-session hook cannot control the
  secure desktop.
* macOS must reuse its injection tag, choose a consistent swallow/repost or
  in-place rewrite strategy, and release all mapped outputs on event-tap
  timeout/user-input disable before re-enabling the tap. Resolve delayed
  tap/hold output without races against capture or configuration changes.
* Keep Ctrl+Alt+Shift+Esc and configured user hotkeys physical, ahead of
  mapping, and route their actions to the daemon. The pure tests include
  remapped Esc/Shift/Ctrl/Alt, but there is no native shortcut wiring yet.

Pure Linux tests cover these adapter decisions, snapshot retirement,
disable/reset, repeat ownership and delayed tap/hold cleanup. The local
random-stream test uses the existing mapper's shared deterministic generator
and down/up oracle (10,000 streams). These do not verify native interception,
injection ordering, timing, AltGr or lock-screen behavior. Windows/macOS
cross-target clippy is compile-only; runtime testing requires those systems.

Groundwork validation (2026-10-29): `cargo test -j 2 --workspace` passed
(648 passed, 6 ignored). `cargo clippy -j 2 --workspace --all-targets --
-D warnings` passed, as did clippy with `--all-targets -- -D warnings` for
`one-mk-input`, `one-mk-proto` and `one-mk-ctl` on
`x86_64-pc-windows-msvc` and `aarch64-apple-darwin` (each with `-j 2`).
No Windows/macOS-specific code was run, no live input backend was started,
and no release build or `cargo fmt` was used.

## Per-device rules (plan 5I)

A rule may be for **one physical keyboard or mouse**: "Caps Lock is Esc on
the small travel keyboard, but stays Caps Lock on the big one", "the
trackball moves at 2x, the mouse at 1x". The rules live in the
`settings.toml` of **the computer the device is plugged into** (by cable,
Bluetooth or a USB receiver) and are applied there, before anything is
sent; the computer being controlled never sees or stores them, and needs
no new version.

```toml
[[input.keymap.rule]]
from = "capslock"
to = "esc"
scope = { device = "bluetooth/04e8:7021/f4:73:35:aa:bb:cc/Travel Keyboard" }

[[input.keymap.rule]]                     # that keyboard, only when it types on the laptop
from = "lalt"
to = "lmeta"
scope = { device = "bluetooth/04e8:7021/f4:73:35:aa:bb:cc/Travel Keyboard", peer = "laptop" }

[[input.pointer]]
speed = 2.0
scope = { device = "usb/047d:2041//Kensington Slimblade Trackball" }
```

### Which devices one-mk can tell apart

| desktop | per-device rules | how | status |
|---|---|---|---|
| Linux, X11 session | yes | XInput2 raw events carry the slave device (`sourceid`); names from `XIQueryDevice`; vendor, product, bus and serial from the XI property `Device Node` and `/sys/class/input/eventN/device/{name,uniq,phys,id/*}` (readable without root); hot-plug from XI hierarchy events | runs; tested against a private Xvfb (only XTEST devices exist there) |
| Linux, GNOME/KDE Wayland (portals), wlroots (Sway, Hyprland, ...) | **no** | the desktop merges every keyboard and mouse before one-mk sees them (libei and the wlroots virtual-input protocols carry no device); one-mk does not read `/dev/input` as root and adds no privileged helper to get around it | not available: says so in `input-devices`, `doctor`, `status` and the window |
| Linux, X11 backend forced under XWayland | **no** | XWayland has only its own merged virtual devices | not available, as above |
| Windows | yes, by correlation | Raw Input (`RIDEV_INPUTSINK`, a message-only window on the hook thread): `WM_INPUT` names the device (`hDevice`), its interface path gives `VID_xxxx&PID_xxxx`, Bluetooth (`BTHENUM`, the HID service `{00001124-...}`) and the interface (`MI_xx`); product and serial from `HidD_GetProductString`/`HidD_GetSerialNumberString`; matched to the hook's event by key/button and time (see below); hot-plug from `WM_INPUT_DEVICE_CHANGE` | **compile-checked only, untested** |
| macOS | yes, by correlation | IOHIDManager on keyboards and mice (VendorID, ProductID, Product, Transport, SerialNumber), input value callbacks matched to the event tap's events by usage and time; needs Input Monitoring, which one-mk already asks for | **compile-checked only, untested** |

### Identity: the fingerprint

A device is known by **what it is**, never by a number the OS hands out
for this session (that changes each time a Bluetooth keyboard sleeps and
reconnects): `bus/vendor:product/serial/name`, for example
`usb/046d:c52b//Logitech USB Receiver` (no serial) or
`bluetooth/04e8:7021/f4:73:35:aa:bb:cc/Travel Keyboard` (the Bluetooth
address as serial). `bus` is `usb`, `bluetooth`, `receiver` (a device
paired to a USB receiver, where the OS shows it as its own device),
`internal` (a laptop's own keyboard and touchpad), `virtual` (made by
software, such as X11's XTEST devices) or `unknown`. A `/` or `%` in the
serial is written `%2F`/`%25`; the name is everything after the third `/`.

**Matching**: a rule whose fingerprint has a serial matches a device with
the same vendor:product and the same serial, whatever its name or bus now.
A rule without a serial matches every device with the same vendor:product
and name (letter case ignored). Hence, honestly:

* **Two identical keyboards without a serial share one rule**: one-mk
  can't tell them apart, and `one-mk input-devices` says so beside them.
* **USB receivers (dongles)**: on Linux each device paired to a Logitech
  style receiver is an input device of its own, with its own name, so a
  rule for the mouse and one for the keyboard on the same receiver work.
  Windows may show every device on a receiver under the receiver's own
  vendor and product (and name): two devices on one dongle may then be
  impossible to tell apart, and `input-devices` says so.
* Plugging a keyboard into another USB port, waking it from sleep, or
  restarting the computer keeps its rules.

### How each event is attributed, and fail-safe

Every key, button, motion and scroll event from the input backend carries
`device: Option<DeviceRef>`: a small number into the table of connected
devices the backend sends as `Event::Devices` at start and on every
change. `None` means **unknown**, and an unknown event gets **only the
rules without a device**, never a device's: a device rule never applies to
an event that might be from another device. Unknown happens on desktops
that merge devices (always), and on Windows and macOS when the two halves
of an event can't be matched in time (see below).

Windows and macOS see an event twice: once in the hook or event tap (which
can swallow it and is what one-mk forwards) and once from Raw Input or
IOHIDManager (which says the device), on different paths, in either order,
a few milliseconds apart. `crates/input/src/devcorr.rs` pairs them:

* a key or button event is matched **in order** with the device events of
  the same key and direction: the oldest one waiting (kept at most 16, for
  50 ms), or the next one to arrive within **8 ms**. Later events wait
  behind it, so nothing is reordered; the only timer runs while one waits;
* unmatched in time, it goes on unknown; after 8 misses in a row nothing
  waits any more (a system that never says costs no latency) until a match
  comes;
* motion is attributed, without waiting, to the one device that moved in
  the last 50 ms; two devices moving, or none, is unknown;
* the race, honestly: two keyboards pressing the same key within a few
  milliseconds are matched in the order the OS delivered each side. If an
  OS delivered the two sides in different orders, they could be swapped;
  if Windows doesn't give Raw Input the events a low-level hook swallows,
  nothing ever matches and all input is unknown (safe, and after 8 misses
  without delay). With the mouse buttons swapped in Windows, clicks may
  stay unknown. None of this has run on real hardware.

On Windows, Raw Input is registered on the capture window only while
capturing (`RIDEV_INPUTSINK`, `RIDEV_DEVNOTIFY`), so nothing extra runs
the rest of the time: the list of devices is read at start, at each
capture start and on `WM_INPUT_DEVICE_CHANGE` while capturing. A device
unplugged while nothing is shared shows as connected in `input-devices`
until the next capture. On macOS, IOHIDManager watches devices come and go
all the time (that costs nothing between events) and their input values
are read only while capturing.

### The mapper with several devices

One `Mapper` engine (no second implementation) holds one compiled keymap
per distinct set of device rules plus the base keymap, chosen per event:
a device with rules of its own uses the base rules with its own rules on
top (per key, as scopes always work); a device without, or an unknown
event, uses the base. The engine keeps one record per **physical key**,
which is a (source, code) pair, a source being one connected device or
"unknown":

* **Every down has exactly one up.** Each physical key remembers the codes
  it emitted (as before) and which source pressed it; its release undoes
  exactly that. The wire state is still derived (holders and hiders per
  code), so two keyboards holding Shift send one Shift down and one up.
* **Modifiers are global**: Shift on keyboard A plus a key on keyboard B is
  Shift+key, and a `ctrl+h` rule fires with Ctrl from either keyboard. A
  tap/hold key on A is decided as held by a key pressed on B.
* **Layers belong to the device that holds the layer key**: holding the
  `nav` layer key on keyboard A changes the keys of A, not of B.
* **Tap/hold timers are per physical key**; there is still at most one
  undecided key at a time (a second press decides the first), so one
  timer at most.
* **Repeats and stray releases**: a press of a key the same source holds is
  a repeat; a press of a key held by the unknown source is that key's
  repeat, whoever it is attributed to now; an unknown press of a key some
  device holds is its repeat. A release goes to the source named if it
  holds the key, else to the unknown source, else to any source holding
  it. So a press and release attributed differently (Windows/macOS) never
  leave a key down.
* **A device disconnects or sleeps mid-hold** (Bluetooth!): everything its
  keys emitted is released at once, its undecided tap/hold key is dropped
  (it had sent nothing), its layers end. Keys whose source was unknown
  are not touched (they might be another keyboard's): they come up with
  their key, at the end of the visit, or with Ctrl+Alt+Shift+Esc.
* **A device appearing or going away changes no other device's state**,
  and a reload only touches the sources whose rules changed: their
  undecided key counts as held and their layers end (as a reload always
  did); other sources keep everything.
* **Visit end, lock, release all**: everything emitted is released, every
  physical key still down is swallowed until it comes up (as before).

Buttons go through the same engine with the same sources. A `Motion`
event's device selects the pointer rules, a scroll event's device the
scroll rules (each device with scroll rules of its own keeps its own
remainders, so notches from that wheel never carry into another's).

### What was tested where

* Pure logic, on Linux: the fingerprint parser (with 200,000 random and
  hostile strings), the mapper with several devices (unit tests and a
  random-stream test over 10,000 streams with three devices, unknown
  attribution, disconnects and per-device reloads, shown to fail with the
  release-on-disconnect or the release fallback removed), the resolution
  of device rules, the remembered list, the Windows path parser, the HID
  usage table and the correlator (with random interleavings).
* Two daemons on loopback (null backends, `app::tests::tuning::per_device_
  rules_follow_the_keyboard_they_come_from`): a rule for keyboard A changes
  A's keys only, unknown input gets only the base rules, A falling asleep
  mid-press releases A's keys on the peer and nothing of B's, A waking with
  another number keeps its rule, pointer speed per device.
* X11 on a private Xvfb (`x11::tests::devices_are_listed_and_input_says_
  which_one`, and the real binaries by hand): only XTEST devices and
  Xvfb's own two exist there, so a second XInput master (with its own
  XTEST keyboard and pointer) stands in for a second keyboard; it is
  listed when added, its keys carry its id, and it leaves the list when
  removed while a key is down (the peer got that key's release).
* Windows and macOS: compile-checked only (`cargo clippy --target
  x86_64-pc-windows-msvc|aarch64-apple-darwin`). Untested, and labelled so
  in `input-devices`, `doctor`, `status` and the window.
* Bluetooth and USB receivers: not tried with real hardware; the sysfs
  reading is tested with a fake sysfs tree.

### Command line and files

* `one-mk input-devices [--json]` lists the keyboards and mice this
  computer has seen: kind, bus, vendor:product, name, serial or address,
  connected now or not and when last seen, and the fingerprint to use. It
  says when per-device rules are not available here and why, and marks
  devices one-mk can't tell apart.
* The list is remembered in `input-devices.toml` in the config folder (at
  most 64 devices, the least recently seen go first; written atomically,
  and only when a device appears or goes away), so a rule can be made for
  a keyboard that is asleep right now. The running daemon writes it; the
  command only reads it.
* `--device NAME|FINGERPRINT` on `keymap add/remove`, `buttons
  map/unmap/preset`, `pointer` and `scroll`: a fingerprint as listed, or a
  name (or a part of one, letter case ignored) that picks exactly one
  device seen; several matches are refused with the candidates listed.
  `--device` combines with `--peer` or `--os` ("this keyboard, when it
  types on the laptop"); `--local --device NAME` stores a rule for this
  computer's keyboard, with Local+Device taking precedence over Local.
  Unknown devices use only Local rules. These rules remain inactive until
  native integration is complete. On `scroll`, `--device wheel` and `--device touchpad` keep
  their old meaning (the kind of scrolling, now also `--kind`); any other
  value names a device. Changing a device rule where devices can't be told
  apart prints a note saying so (the rule is kept).
* Going back to a one-mk from before 5I: it can't read a `settings.toml`
  with rules for one device (its scopes know no `device`), and ignores the
  whole file. Remove them first (`--device` on the remove commands).
* `one-mk doctor` has a **Per-device rules** check: pass where devices can
  be told apart (Windows and macOS say untested), skip with the reason on
  desktops that merge them. `one-mk status` says when device rules are set
  but can't apply here.

## The key mapper (`crates/proto/src/remap.rs`)

Pure logic, no I/O, no allocation per event, O(1) per event (a layer
release walks the fixed-size held-key bitset, 12 words).

### Data model

* **Key names** (`crates/proto/src/keynames.rs`): one table of evdev codes,
  each with its canonical name and aliases, case-insensitive: `esc`,
  `capslock`, `lctrl`/`ctrl`/`control` (left), `rctrl`, `lshift`/`shift`,
  `lalt`/`alt`/`option`, `ralt`/`altgr`, `lmeta`/`meta`/`super`/`win`/`cmd`,
  `f1`..`f24`, `kp0`..`kp9`, `a`..`z`, `0`..`9`, punctuation by name or
  character (`minus` or `-`), media keys by their names in `mediakeys.rs`
  (`volume up`, `volume-up` and `volumeup` are the same), mouse buttons
  `button1`..`button8`, and raw `code:58` or `evdev:KEY_CAPSLOCK`. The
  parser and printer are shared by the CLI, `settings.toml` and (later) the
  window. Bad input is an error saying which word is wrong, never a panic.
* **Trigger** (`from`): a key, optionally with modifiers that must be held:
  `h`, `ctrl+h`, `lctrl+shift+h`. In a trigger `ctrl` means *either* Ctrl,
  `lctrl`/`rctrl` one side. The rule with the most matching modifiers wins;
  other modifiers held pass through (`ctrl+shift+h` with a `ctrl+h ->
  backspace` rule gives Shift+Backspace).
* **Output** (`to`, `tap`, `hold`):
  * a key or chord, held as long as the key is (`esc`, `ctrl+shift+esc`);
    in an output `ctrl` means the left one;
  * a sequence, an array (`["ctrl+a", "ctrl+c"]`), typed once on press
    (at most 16 steps);
  * `none` (the key does nothing);
  * `layer:NAME`, the layer is on while the key is held;
  * `back`, `forward`: browser back/forward for the target OS (Alt+Left/
    Alt+Right, on a Mac Cmd+[ / Cmd+]);
  * an action, run once on press: `switch next|here|NAME`, `lock-pointer
    [toggle|on|off]`, `lock-all`, `scroll up|down|left|right [N]`, `run:
    COMMAND` (no shell; split like `[hooks]` commands, the same timeout,
    rate limit and running-count limit: it goes through `hooks.rs`).
* **Rule**: `from` plus `to`, or `tap` and `hold` (dual-role), optionally
  `layer = "NAME"` (the rule only applies while that layer is on) and
  `timeout_ms` (dual-role, default `[input.keymap] tap_ms`, 200 ms).

### State machine

Per physical key the mapper keeps a small record of what its press
produced: nothing (swallowed), a held output (a key, the modifiers it
added, the modifiers it hid, and the layer it was resolved in), a layer
it switched on, or "pending" (a dual-role key not yet decided). The key's
release undoes exactly that record, *whatever the configuration, layers or
modifiers are by then*. The record holds codes, not references into the
configuration, so a reload cannot change what a release does.

What goes on the wire is derived, never tracked by hand: every output code
has a count of holders, every modifier a count of "hiders" (combos hide the
modifiers they were triggered with, `ctrl+h -> backspace` must not send
Ctrl+Backspace). A code is down on the wire iff it has a holder and is not
hidden. The mapper only emits a transition when that derived state
changes, and keeps the wire state in a bitset. So:

* every key-down sent has exactly one key-up, by construction;
* two sources holding one code (Caps Lock as Ctrl while the real Ctrl is
  held) send one down and one up;
* **release all** (visit end, disconnect, lock, reload) releases the wire
  bitset, which is what the mapper emitted, not what is physically held;
  physical keys still down are marked swallowed until they come up;
* **auto-repeat**: a repeated down of a key whose record holds an output
  repeats that output's key (if it is down on the wire); a pending,
  swallowed or layer key ignores repeats; sequences and actions never
  repeat;
* **typed once** (a tap, a sequence step) produces exactly one fresh key
  press, even if something already holds that key. A held key is released
  before its first pulse; intermediate pulses go down/up, and the final
  pulse's down also restores the original holder (no extra press after
  each step). Thus `["a", "a"]` with A held produces exactly two A-down
  transitions, remains held afterward, and is released by its original
  holder. The chord's modifiers are down even while a combo hides them,
  then back as they were;
* **actions** are emitted as the action itself (shared with the keymap,
  no copy), not an index into the keymap, so one emitted just before a
  reload still runs as the old keymap had it.

**Dual-role** (`tap`/`hold`): pressing the key makes it pending and arms a
deadline. It resolves to *hold* when the deadline passes, when another key
is pressed while it is down ("permissive hold": the other key then sees the
hold, so Caps-as-Ctrl + C is Ctrl+C), or when a mouse button is pressed or
the wheel turned; to *tap* when it is released first (tap down and up are
sent at once). Every event first checks the deadline, so a release that
arrives after it, before the timer ran (a busy loop), is a hold, not a tap. The deadline is the mapper's only timer: the daemon's event
loop includes it only while a key is pending, and it fires or is cancelled
exactly once, so an idle keyboard costs nothing.

**Layers**: `to = "layer:nav"` (or `hold = "layer:nav"`) switches the
layer on while the key is down; rules with `layer = "nav"` then apply,
the most recently switched-on layer first, falling through to the layers
under it and then the base rules for keys a layer doesn't map. When the
layer key comes up, every key resolved in that layer is released at once
(and swallowed until it physically comes up), so `h` held as Left in the
nav layer can't stay held after the layer ends.

**Reload mid-visit**: an identical keymap changes nothing. A different one
first resolves a pending key as hold, ends every layer (releasing what was
typed in it), and keeps the other held outputs held until their keys come
up; new presses use the new rules.

**Modifier-scroll** asks the mapper to hide a modifier until it is
released (see scrolling).

### Limits (checked when the rules are compiled)

At most 8 layers, 16 steps in a sequence, 4 modifiers plus a key per
chord, 4096 rules. Codes are evdev codes below 0x300 (keys and
mouse buttons share the space). A dual-role key can't have modifiers in
`from` and `tap` can't be a layer.

## Pointer tuning (`crates/proto/src/pointer.rs`)

* `speed`: 0.1 to 10, multiplies every delta (the remote pointer moves
  `speed` times as far). Applied in `f64`; nothing is rounded, so a million
  small moves add up exactly (tested).
* `acceleration`:
  * `system` (default): the deltas are what this computer's OS made of the
    mouse movement, acceleration included. One-mk sends *positions*
    computed on this computer, so this computer's acceleration applies once
    and the target's never does.
  * `off` and `curve` need **unaccelerated** deltas. Acceleration can't be
    divided back out of accelerated motion (the OS curve, its state and its
    input are unknown), so these work only where the input backend reports
    raw deltas, and **no backend does yet**:

    | OS | what the backend reads | raw deltas possible? |
    |---|---|---|
    | Linux (Wayland, libei) | the compositor's pointer motion, already accelerated | no: libei carries one delta and compositors fill it with the accelerated motion |
    | Windows | the low-level mouse hook (cursor positions) | yes, with Raw Input (`WM_INPUT`); not wired yet |
    | macOS | `CGEvent` mouse deltas, accelerated | only through IOHIDManager (another permission); not wired yet |

    Until then `one-mk pointer --accel off|curve` refuses with this
    explanation and how to turn acceleration off in the OS instead (GNOME:
    Settings, Mouse, Mouse Acceleration off; KDE: acceleration profile
    flat; Windows: Mouse, Additional mouse settings, Enhance pointer
    precision off; macOS: Mouse, Pointer acceleration off on macOS 14+).
    That setting then holds for every computer this one controls. A
    hand-written `off`/`curve` in `settings.toml` is reported by `one-mk
    doctor` and treated as `system`.
  * `curve` (for when raw deltas exist): `gain = 1 + accel_gain *
    |delta|^accel_exp`, capped at `accel_cap` (|delta| in counts per report).
    The module implements and tests it.

## Scroll tuning (`crates/proto/src/scroll.rs`)

Per scope and optionally per device kind (`device = "wheel"` for notched
wheels, which arrive as `ScrollDiscrete`, `"touchpad"` for smooth scrolling;
leave it out for both). Each event goes through: direction (of the wheel
or fingers, so a reversed wheel stays reversed when Shift turns it
sideways), modifier behaviour, speed, then mode.

* `reverse_vertical`, `reverse_horizontal`: flip that axis.
* `natural = on|off|auto`: what the target sees. One-mk assumes scroll
  arrives from this computer the way its OS delivers it by default
  (macOS: natural, content follows the fingers; Windows and Linux:
  traditional). By default (and with `natural` unset) scrolling feels on
  the target exactly as it does here. `on` makes it natural on the target,
  `off` traditional, `auto` follows the target OS's default (natural on a
  Mac, traditional elsewhere): from a Mac to a PC, auto flips; from a PC
  to a Mac, auto flips; same OS family, nothing changes. If this computer's
  own setting is not its OS default, use `reverse_*` instead.
  `natural` is combined with `reverse_*` (both flip).
* `speed`: multiplier per axis (`speed`, or `speed_x`/`speed_y`), 0.1 to 10.
* `mode`: `auto` (default: as it comes), `smooth` (wheel notches become
  pixels, 48 px a notch like the Windows backend), `discrete` (pixels and
  partial notches add up in an accumulator and go out as whole notches:
  "scroll by lines"). Remainders are kept, so nothing is lost or invented
  over any number of events (tested over a million); a `ScrollStop`
  (fingers lifted) clears the partial notch.
* modifiers: `modifiers = { shift = "horizontal", alt = "x5", ctrl =
  "unchanged" }`; behaviours `horizontal` (vertical scrolling becomes
  horizontal), `vertical` (horizontal is dropped), `xN` (multiply by N,
  0.1 to 50), `unchanged`. The modifiers are the mapper's (so a key mapped
  to Shift counts). **The modifier is forwarded until it is used to
  scroll**: Shift goes to the target when pressed (Shift+click must still
  work), and the first tuned scroll lifts it there (a key-up is sent) and
  keeps it lifted until it is released here. The target therefore never
  sees Shift+horizontal-scroll, and never keeps Shift held. (A key typed
  after scrolling while Shift is still held is then unshifted.)

## Button remap

Buttons are evdev codes in the same space as keys, so they go through the
same mapper, with the same pairing guarantee (a click-and-hold or drag
mapped to another button keeps it held until the physical button comes
up): `[[input.button]] from = 4, to = "back"`. Numbers follow the usual
convention: 1 left, 2 right, 3 middle, 4 back (side), 5 forward (extra),
6 to 8 the evdev `BTN_FORWARD`, `BTN_BACK`, `BTN_TASK`. A button can become
another button, a key, chord or sequence, an action, `back`/`forward`
(resolved per target OS: the preset `one-mk buttons preset
side-back-forward` maps 4 and 5 so), or `none`. Opt-in middle-click
autoscroll is described in [5F](#autoscroll-and-gesture-buttons-5f).

## Configuration

```toml
[input.keymap]
tap_ms = 200                       # dual-role decision time

[[input.keymap.rule]]
from = "capslock"
tap = "esc"
hold = "lctrl"

[[input.keymap.rule]]
from = "lalt"
to = "lmeta"
scope = { os = "mac" }

[[input.keymap.rule]]
from = "ralt"
to = "layer:nav"

[[input.keymap.rule]]
from = "h"
to = "left"
layer = "nav"

[[input.keymap.rule]]
from = "f13"
to = "switch next"

[[input.pointer]]
speed = 1.5
scope = { peer = "laptop" }

[[input.scroll]]
natural = "auto"
modifiers = { shift = "horizontal", alt = "x5" }

[[input.scroll]]
device = "wheel"
mode = "discrete"
speed = 2.0
scope = { os = "windows" }

[[input.button]]
from = 4
to = "back"

[[input.keymap.rule]]              # only while VS Code has the focus on a Mac
from = "ctrl+space"
to = "f13"
scope = { os = "mac" }
app = "com.microsoft.VSCode"

[[input.pointer]]                  # slower in games on any computer
speed = 0.6
app = "*game*"
```

Values are validated when loaded; out-of-range numbers are clamped and
reported by `one-mk keymap check` and `one-mk doctor`.

## Command line

* `one-mk keymap [list]`, `keymap add FROM TO [--tap K --hold K] [--layer L]
  [--peer P | --os O] [--app NAME]`, `keymap remove FROM [--layer L]
  [--peer P | --os O] [--app NAME]`, `keymap clear`, `keymap check`.
* `one-mk pointer [--speed X] [--accel system|off|curve] [--peer P | --os O]
  [--app NAME] [--device D]`; `--device D` (a keyboard or mouse, see
  "Per-device rules") works on every command here and combines with
  `--peer` or `--os`.
* `one-mk scroll [--reverse-vertical on|off] [--reverse-horizontal on|off]
  [--speed X] [--mode auto|smooth|discrete] [--natural on|off|auto]
  [--modifier shift=horizontal]... [--kind wheel|touchpad] [--peer P |
  --os O] [--app NAME]`.
* `one-mk buttons [list]`, `buttons map N TO [--peer P | --os O] [--app
  NAME]`, `buttons unmap N [...]`, `buttons preset side-back-forward`,
  `buttons clear`.

`--app` is checked like `app =` in the file; with `--os`, a name that
can't occur on that OS (`chrome.exe` for a Mac) is refused. `keymap list`,
`buttons list`, `pointer` and `scroll` show it ("only in app code").
`keymap check` also reports an app that never matches on its rule's OS
(by `--os`, or the paired computer's OS for `--peer`), and checks each
plainly named app's rules together (conflicts, layers).

Each shows the current values when run bare, changes `settings.toml`
through `Settings::update` (file lock), refuses invalid input with a
precise message, and applies at once through the running daemon (`Reload`).
`one-mk status` adds one line when any rule is set, and one per computer
whose rules for apps aren't in force (see above); `one-mk doctor` has an
"Input tuning" check (skip when nothing is set; warns about rules that can
never apply, conflicting rules for one key in one scope, unknown computers
in scopes, apps that can't match, and acceleration modes this computer
can't do; with rules for apps and a Linux computer paired, it notes that
Wayland windows there don't say which app they are).

## Autoscroll and gesture buttons (5F)

These optional modes affect **only input sent to another computer**. Both
are off until configured. Physical hotkeys, including Ctrl+Alt+Shift+Esc,
run first. There is no protocol change.

```
one-mk autoscroll --on
one-mk autoscroll --speed 1.5 --dead-zone 8 --vertical-only off
one-mk autoscroll --on --button middle --os windows
one-mk autoscroll --off --app 'code*'
one-mk autoscroll --off
one-mk gestures list
one-mk gestures add button4 left back --threshold 60
one-mk gestures add button5 right 'switch next'
one-mk gestures add middle up 'ctrl+super+left' --os windows
one-mk gestures add button4 down ctrl+a ctrl+c --app 'code*'
one-mk gestures preset side-swipe
one-mk gestures remove button4 left
one-mk gestures clear
```

**Autoscroll:** click middle once to start. Moving away from the anchor
scrolls continuously; the target pointer stays at the anchor. Return inside
the dead zone to pause. Click again, press Esc or any other button, or leave
the visit to stop. The toggle click is never sent as a middle click; other
buttons stop scrolling and still work normally. Speed is a multiplier
(0.1–10); dead zone is backend motion units (0–1000). Both axes work unless
`vertical_only` is on. Per-axis rate is
`min((max(abs(displacement) - dead_zone, 0) / 20)^1.5, 100) * speed`
smooth-scroll units per 16 ms tick, with the displacement's sign. Speed/dead-zone
configure its response; the exponent is fixed. Late ticks never catch up
in bursts. The cancelling physical Esc press (including repeats and release)
is consumed, so it cannot close a remote dialog. An Esc already held before
the mode started still releases normally. Esc generated by a key remap is
ordinary remote input, not a mode command; use the physical Esc key to cancel.

```toml
[input.autoscroll]
enabled = true
button = "middle"
speed = 1.0
dead_zone = 8.0
vertical_only = false

[[input.autoscroll.rule]]
scope = { os = "windows" }
speed = 1.5

[[input.autoscroll.rule]]
app = "code*"
enabled = false

[[input.gesture]]
button = "button4"
direction = "left"
to = "back"
threshold_px = 60.0
scope = { peer = "Studio PC" }
app = "firefox*"
```

Autoscroll overrides use `[[input.autoscroll.rule]]`, with the same `scope`
and `app` machinery as other tuning: computer beats OS beats global; app
matches outrank rules without apps. Fields inherit independently. CLI
selectors write overrides; no selector writes the base table. Gesture
rules resolve per button/direction, most specific first. `remove`/`clear`
affect only the selected scope/app, global by default. `add` replaces the
same button/direction in that scope/app.

**Gestures:** hold the configured button, then move beyond its threshold in
a configured direction. The action fires once per press. Release before
moving beyond the click slop to replay exactly one normal, **unmapped**
click at the unchanged target pointer position. Slop is the smaller of 16
backend motion units and half the smallest threshold on that button.
Reaching that displacement on either axis permanently suppresses click replay
for that press, including a wrong-direction or missed swipe that returns to
the origin. This prevents accidental browser Back after an attempted swipe. Other
configured gesture-button presses and all movement while holding them are swallowed.
Physical Esc, relevant rule changes, display sleep, locking, disconnect and visit end cancel
without replay. Threshold is 1–10000 backend motion units (default 60).
Diagonal movement selects the larger axis, vertical on a tie.

`threshold_px` retains its historical config name for compatibility; it is
**not** a cross-platform logical-pixel measurement. Input is the backend's
delta before one-mk pointer tuning: X11 valuators, Windows hook deltas, and
macOS points. Display scaling and OS acceleration can change the feel; tune
the threshold/dead zone on each controller. No hardware equivalence is claimed.

Left/right primary buttons (`button1`/`button2`, including aliases) are refused:
they must remain usable for clicks and drags. Invalid existing rules are ignored
and reported by `keymap check`, doctor, and the Mouse view. Per-device autoscroll
and gesture rules are also ignored with a warning; select a computer/OS/global
scope instead. The window hides their controls when For: selects a device and
explains how to change scope. Gesture direction removal is case-insensitive.

Broadcast visits do **not** enable either mouse mode: middle and side clicks
are forwarded through ordinary button mappings, and no autoscroll deadline is
armed, including after a reload. Normal visit transport keepalives still exist.

Outputs reuse the key mapper's parser and dispatch: keys, chords,
sequences (`to = ["ctrl+a", "ctrl+c"]`), media keys, `back`/`forward`,
`switch next`, `lock-all`, and `run: COMMAND`. Command outputs have the
same file-permission checks as key actions. Layer holds are rejected:
gestures pulse actions rather than hold layers. Navigation, modifiers and
Ctrl/Cmd swapping follow the target OS. The side-swipe preset uses button4
and `desktop-left`/`desktop-right`: Ctrl+Super+arrow on Windows, Ctrl+arrow
on macOS, Ctrl+Alt+arrow on Linux. If the target desktop uses different
shortcuts, configure those chords instead.

Precedence is **autoscroll toggle → gesture button → button/key mapping**.
Autoscroll owns its button completely; gestures on that button are skipped.
Gesture ownership also overrides modifier/layer button mappings. Checks
warn about overlaps. Short clicks replay the original button, not its
`buttons map` output. Gesture pulses share the key mapper's held-state
accounting, so they cannot release another key's held output.

The existing visit deadline scheduler arms one 16 ms wakeup **only while
autoscroll is active**, including pauses inside the dead zone. No extra
task/thread/idle interval is created. `ScrollTune` applies smooth/notched
conversion, direction, speed and modifiers; fractional notches accumulate
until stop and are then discarded. A change to resolved autoscroll settings
cancels its timer; a change to resolved gesture rules cancels a held gesture.
Unrelated saves, device hotplug, and unchanged focused-app resolution preserve
each mode independently. Visit end cancels both;
restart/reconnect never restores active modes. A controller display-sleep
notification cancels both modes without changing the existing sleep-sync
policy or ending the visit.

The **Keys & mouse → Mouse** view adds an Autoscroll toggle/speed, a
gesture list with Remove controls and the side-swipe preset. Existing focus
rings, keyboard-operable controls, contrast palette and reduced-motion
policy apply. Advanced button, threshold, app and output editing uses the
CLI/config.

### Troubleshooting and acceptance

Run `one-mk keymap check` and `one-mk doctor`. If a button stops clicking,
check autoscroll/gesture ownership before `buttons map`. If scrolling
continues, press Esc or switch here. If nothing scrolls, ensure you're
visiting a peer, move beyond the dead zone, and check that its app scrolls.

**Not hardware-qualified:** automated tests use Linux null backends and
real loopback links. Windows/macOS are compile-checked only. Native
GNOME/wlroots capture/injection, Windows/macOS runtime, Bluetooth devices
and real desktop shortcuts are untested. Status/doctor repeat this boundary.
The [5F verification record](5F-VERIFICATION.md) lists real executable and
window checks, measurements, and the pre-existing portal-test blocker.

Manual hardware acceptance, using disposable settings on each target OS:

1. Enable autoscroll, visit a page, click middle and move on both axes.
   Expect scrolling, no target pointer movement, no middle-click action.
   Return to the anchor: no drift. Stop separately with middle/Esc/another
   button; the other button must still click.
2. Use discrete scroll mode and tiny displacements: complete notches
   accumulate; no delayed notch after stop or another visit.
3. Configure a swipe. A short click must click once; a long swipe must
   fire once and never click. Hold other mapped keys while swiping, release
   in either order, and test the emergency chord. Nothing stays held.
4. Mid-mode, switch here, reload (including changing the reserved button),
   lock, sleep, disconnect, and kill the sender. The target must release
   everything; reconnect must not resume old modes.
5. Check the Mouse view narrow/HiDPI, reduced motion, many/long rules,
   offline/errors, and Tab/Enter/arrow-key operation and focus labels.

## Limits per OS

* **Only input sent to other computers is tuned.** On Linux (Wayland) the
  daemon sees the keyboard and mouse only while it forwards them to a
  peer (the InputCapture portal), so the rules can't change local typing,
  and nothing short of a root-level evdev remapper (keyd, kanata) could.
  Windows (low-level hooks) and macOS (event taps) could remap local input
  too; plan 4C currently has only the inactive config/CLI and pure adapter
  groundwork described above, not a working native integration.
* Pointer acceleration `off`/`curve`: see the table above.
* Natural scrolling assumes this computer uses its OS default direction.
* Per-app rules: see the table under "Per-app rules"; on GNOME and KDE
  Wayland only X11 (XWayland) windows can be named.

## The window (4B)

The **Keys & mouse** tab (`crates/app/src/app/keys.rs`; everything that
isn't drawing is in `crates/app/src/tune.rs` and unit-tested) reads
`[input]` from `settings.toml` with the window's own small reader and
changes it only through the commands below, so the window and the command
line can't disagree. It uses:

* names: `one_mk_proto::keynames::{parse_key, key_name, Chord, Trigger}`
  (`FromStr` + `Display`, round-trip), `one_mk_proto::remap::Output`
  (`FromStr` + `Display`), `one_mk_proto::keynames::all_names()` for the
  searchable list, `one_mk_proto::keynames::button_number`/`button_code`
  for buttons;
* changes: `one-mk keymap add|remove`, `one-mk pointer ...`, `one-mk scroll
  ...`, `one-mk buttons map|unmap|preset` (validated, locked, applied live),
  run on one worker thread, one command at a time; options come first and
  then `--`, so a key or computer name is never read as an option. Before a
  rule is sent, the window makes the same checks as `one-mk keymap add`
  (the rule reads, it isn't a mouse button, a tap/hold key builds alone, the
  whole keymap still builds) and shows the message inline;
* problems: `one-mk keymap check` (exit status 1 and one line per problem),
  run after every change and shown above the rule list;
* per-app rules (4D): a rule's sentence ends with its app ("Caps Lock →
  Ctrl, only in Code", "…, in every app except code"); the key editor has
  an **Only in app** field (empty: every app). Typing an app there moves
  the key's rule to that app, or, with no rule yet, makes the next change
  for that app. The drawn keyboard and the Test it box show the rules for
  every app (the focused app isn't known in the window);
* per-device rules (5I): **For:** has a section **One keyboard or mouse**
  listing the devices from `one-mk input-devices --json` (name, then
  "Connected" or "Asleep or unplugged", connection and kind; devices made
  by software are left out), fetched each time the menu opens. Where the
  desktop merges devices it says so in plain words instead; with none seen
  yet it says how they appear. Rules for a device with a computer too
  (from the command line) are read and shown as such ("Only Travel
  Keyboard, on laptop");
* the **Test it** box: the rules that apply to the computers chosen in
  **For:**, resolved like the daemon does (a computer beats an OS beats
  everything) and compiled with `remap::Keymap::compile`, go through a
  `remap::Mapper` in the window. Keys typed there show what would be sent
  (actions are shown, never run) and how a tap/hold key was decided (let go
  within the time, held past it, or another key pressed). The mapper's only
  timer runs while a tap/hold key is undecided and the window is drawing.

Keys are captured from egui's key events by their physical position
(`physical_key`) and mapped to evdev codes (`tune::evdev_of`). egui can't
report some keys, so those are picked from the list instead: the keypad
(egui folds it into the main keys: Keypad 7 arrives as 7), Fn (usually
handled inside the keyboard), the Japanese and Korean keys, brightness,
keyboard light and microphone keys, F25 and up, and keys the desktop keeps
for itself (Super on GNOME, Print Screen, often the media keys). Ctrl+C,
Ctrl+X and Ctrl+V reach egui as Copy, Cut and Paste and are read as C, X
and V. Esc cancels a capture (to make a key become Esc, pick Esc from the
list).

Ctrl+Alt+Shift+Esc is matched before the mapper, so it always works; the
tab says so under the test box.
