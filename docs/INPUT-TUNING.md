# Input tuning and the key mapper (plan 4A)

What LinearMouse does for one Mac, for every computer one-mk sends input
to: change any key, tap/hold keys and layers, pointer speed and
acceleration, scroll direction, speed and mode, modifier-scroll, and mouse
button remapping. Rules apply to everything, to computers of one OS, or to
one computer, so the same mouse can scroll "naturally" on the Mac and
traditionally on the PC.

## Where it runs

Everything runs on the **controller** (the computer the mouse and keyboard
are plugged into), in the send path of a visit, before anything reaches
the network. The computer being controlled receives ordinary keys,
buttons, positions and scrolling, so it needs no new version and nothing
about it changes: an older peer gets remapped input like any other.

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

`peer` beats `os` beats `global`. Rules are resolved when a visit starts
and again when `settings.toml` is reloaded. A rule may also carry `app =
"..."`: reserved for per-app rules (plan 4D); such a rule is not used yet,
and `one-mk keymap check` says so.

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
side-back-forward` maps 4 and 5 so), or `none`. Middle-click autoscroll
("scroll lock") is not implemented.

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
```

Values are validated when loaded; out-of-range numbers are clamped and
reported by `one-mk keymap check` and `one-mk doctor`.

## Command line

* `one-mk keymap [list]`, `keymap add FROM TO [--tap K --hold K] [--layer L]
  [--peer P | --os O]`, `keymap remove FROM [--layer L] [--peer P | --os
  O]`, `keymap clear`, `keymap check`.
* `one-mk pointer [--speed X] [--accel system|off|curve] [--peer P | --os O]`.
* `one-mk scroll [--reverse-vertical on|off] [--reverse-horizontal on|off]
  [--speed X] [--mode auto|smooth|discrete] [--natural on|off|auto]
  [--modifier shift=horizontal]... [--device wheel|touchpad] [--peer P |
  --os O]`.
* `one-mk buttons [list]`, `buttons map N TO [--peer P | --os O]`, `buttons
  preset side-back-forward`, `buttons clear`.

Each shows the current values when run bare, changes `settings.toml`
through `Settings::update` (file lock), refuses invalid input with a
precise message, and applies at once through the running daemon (`Reload`).
`one-mk status` adds one line when any rule is set; `one-mk doctor` has an
"Input tuning" check (skip when nothing is set; warns about rules that can
never apply, conflicting rules for one key in one scope, unknown computers
in scopes, and acceleration modes this computer can't do).

## Limits per OS

* **Only input sent to other computers is tuned.** On Linux (Wayland) the
  daemon sees the keyboard and mouse only while it forwards them to a
  peer (the InputCapture portal), so the rules can't change local typing,
  and nothing short of a root-level evdev remapper (keyd, kanata) could.
  Windows (low-level hooks) and macOS (event taps) could remap local input
  too; that is plan 4C.
* Pointer acceleration `off`/`curve`: see the table above.
* Natural scrolling assumes this computer uses its OS default direction.

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
