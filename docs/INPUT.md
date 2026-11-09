# Keys, mouse and scrolling

What `one-mk keymap`, `buttons`, `pointer`, `scroll`, `autoscroll`,
`gestures`, `typing` and `input-devices` do, with examples and the limits of
each. The window's **Keys & mouse** tab and **Settings** tab run these same
commands, so the window and the command line always agree.

Rules apply to **input sent to the other computers**: your keyboard and mouse
are read on the computer they are plugged into, changed there, and then sent.
The computer being controlled needs nothing new. Your own typing on your own
computer is not changed (except by the experimental local rules, below).
Every command changes `settings.toml` and, if one-mk is running, applies at
once. Run any of them with no arguments to see what is set.

## Which computers a rule is for

| option | the rule applies to |
|---|---|
| (none) | every computer you control |
| `--os mac`, `--os windows`, `--os linux` | computers running that system |
| `--peer NAME` | one paired computer, by name or id |
| `--app NAME` | only while that app has the focus on the computer being controlled |
| `--device NAME` | only input from one keyboard or mouse (see "One keyboard or mouse"); combines with `--peer` or `--os` |

The most specific rule wins: device, then computer, then system, then
everything; a rule for the focused app beats every rule without an app.
`--app` names are a program file on Windows (`chrome.exe`), a bundle id on a
Mac (`com.apple.Terminal`), a window class on Linux (`firefox`); `*` matches
anything and a leading `!` means every app but that one.

## Keys: `keymap`

```
one-mk keymap add capslock esc                      # Caps Lock becomes Esc
one-mk keymap add capslock --tap esc --hold lctrl   # tap for Esc, hold for Ctrl
one-mk keymap add ctrl+h backspace
one-mk keymap add lalt lmeta --os mac               # only toward Macs
one-mk keymap add f13 "switch next"                 # an action
one-mk keymap add ralt layer:nav                    # a layer: on while held
one-mk keymap add h left --layer nav
one-mk keymap add f14 "run: notify-send hello"      # run a command
one-mk keymap remove capslock
one-mk keymap list
one-mk keymap check                                 # every problem, exit status 1 if any
one-mk keymap clear
```

A key can become another key, a chord (`ctrl+shift+esc`), a sequence
(`ctrl+a ctrl+c`), `none` (it does nothing), `layer:NAME`, `back` or
`forward` (as the target system spells them), or an action: `switch next`,
`switch here`, `switch NAME`, `lock-pointer`, `lock-all`, `scroll down`,
`run: COMMAND`. Tap/hold keys decide after 200 ms by default (50 to 2000; set
`tap_ms` under `[input.keymap]` in `settings.toml`, or `--timeout-ms` on one
rule).

* `run:` commands start programs, so they are only run when the settings
  folder cannot be changed by other users (no group or world write access),
  like `hooks`. If it can, the rules are saved but refused: `keymap add`
  says so, and `keymap check` and `doctor` list it.
* **Ctrl+Alt+Shift+Esc** is one-mk's emergency chord. It is checked before
  any rule, always works, and cannot be remapped.
* A rule in a layer that no key switches on can never work; `keymap add`
  and `keymap check` say so.
* Keys held when a rule changes, a computer is switched or a connection
  drops are released as the keys that were really sent: nothing stays stuck.

## Mouse buttons: `buttons`

```
one-mk buttons preset side-back-forward   # buttons 4 and 5 go back and forward
one-mk buttons map 4 back
one-mk buttons map 3 ctrl+w
one-mk buttons map 5 none --app "code*"
one-mk buttons unmap 4
one-mk buttons list
```

Buttons are numbered 1 left, 2 right, 3 middle, 4 back, 5 forward, 6 to 8
extra. A button can become another button, a key, a chord, a sequence, `back`
or `forward`, an action, or `none`. **Left and right cannot be made to do
nothing or hold a layer** (the other computer would have no clicks); they are
refused, and `keymap check` flags such a rule written by hand.

## Mac and PC keyboards: Ctrl and Cmd

When keys go between a Mac and a Windows or Linux computer, Ctrl and Cmd trade places by default, so
Ctrl+C on the PC keyboard copies on the Mac and the other way round. The sending computer does the swap,
after any key rules. Set it per paired computer:

```
one-mk layout "Mac" --swap-ctrl-cmd auto     # the default: swap only between a Mac and something else
one-mk layout "Mac" --swap-ctrl-cmd on|off
```

It is stored for each computer on the computer where you set it (the window's **Settings > Ctrl and Cmd**
card does the same).

## Pointer speed: `pointer`

```
one-mk pointer --speed 1.5
one-mk pointer --speed 0.6 --app "*game*"
one-mk pointer --speed 2 --peer Laptop
one-mk pointer --reset --peer Laptop
```

Speed is a multiplier from 0.1 to 10. Pointer **acceleration** cannot be
changed from one-mk yet (it needs raw mouse motion that no backend reports).
Turn it off in the operating system instead (GNOME: Settings, Mouse &
Touchpad; KDE: Acceleration profile Flat; Windows: Enhance pointer
precision; macOS: Pointer acceleration). One-mk sends positions computed on
your computer, so that setting holds on every computer you control. A
leftover `acceleration` entry in an old `settings.toml` is read and ignored.

## Scrolling: `scroll`

```
one-mk scroll --natural auto               # natural on a Mac, traditional elsewhere
one-mk scroll --reverse-vertical on
one-mk scroll --speed 2 --mode discrete    # whole notches
one-mk scroll --modifier shift=horizontal --modifier alt=x5
one-mk scroll --kind wheel --speed 1.5     # wheels only (or --kind touchpad)
one-mk scroll --reset --kind touchpad
```

`--mode` is `auto` (as it comes), `smooth` (pixels) or `discrete` (partial
notches add up and go out as whole notches, nothing lost). A modifier can turn
vertical scrolling horizontal (`horizontal`), drop sideways scrolling
(`vertical`), or multiply (`x0.1` to `x50`); the modifier is not sent to the
other computer while it is used to scroll. `--natural` assumes this computer
uses its system's default direction; otherwise use `--reverse-*`.

## Autoscroll and gestures

Both are off until set up, work only toward another computer, and are not
yet tested on real hardware (Linux null-backend tests only; Windows and
macOS are compile-checked).

```
one-mk autoscroll --on                     # click middle, move away: it scrolls
one-mk autoscroll --speed 1.5 --dead-zone 8 --vertical-only off
one-mk autoscroll --off --app "code*"
one-mk autoscroll --reset --app "code*"    # forget that override
one-mk gestures preset side-swipe          # hold the back button, swipe: change desktop
one-mk gestures add button4 left back --threshold 60
one-mk gestures add middle up "ctrl+super+left" --os windows
one-mk gestures list
one-mk gestures remove button4 left
```

* Autoscroll: click the toggle button (middle by default) and move away; the
  further you move, the faster it scrolls. Click again, press Esc or any other
  button, or leave the computer to stop. The toggle click is not sent.
* Gesture: hold the button and move past the threshold (1 to 10000 motion
  units, 60 by default) in a direction; the output happens once. A short
  click still clicks. Esc cancels a swipe. `desktop-left` and `desktop-right`
  switch virtual desktops on any system.
* The toggle button of autoscroll belongs to autoscroll: a gesture on it, or
  an autoscroll button that a gesture already uses, is refused. Left and
  right buttons cannot be used for either.
* Autoscroll and gestures apply to the pointer of a visit, so they cannot be
  set for one device.

## Typing: `typing` (letters are experimental)

```
one-mk typing                              # what is set, and what each computer will do
one-mk typing --keys characters            # experimental
one-mk typing --keys auto --peer Laptop
one-mk typing --keys physical
```

By default (**physical**; "Keys" in the window) each key you press is the
same key on the other computer, so letters follow *its* keyboard layout.
`characters` ("Letters") sends the letter printed on your key instead, and
`auto` does so only when the two layouts differ. **Letters and Auto are
experimental:** a receiving Windows or Mac computer can type them, but this has
not been tested on real hardware; a Linux receiver cannot type letters yet and
keeps using physical keys, so on Linux-to-Linux the setting changes nothing.
Shortcuts always use the keys. `typing` prints what each paired computer will
really do.

## One keyboard or mouse: `--device`, `input-devices`

```
one-mk input-devices                       # keyboards and mice this computer has seen
one-mk keymap add capslock esc --device "Travel Keyboard"
one-mk pointer --speed 2 --device "Trackball"
```

Rules for one device are stored on the computer the device is plugged into.

| desktop | per-device rules |
|---|---|
| Linux, X11 session | work (tried against a private X server) |
| Linux, GNOME, KDE or wlroots on Wayland | **impossible**: the desktop merges every keyboard and mouse before one-mk sees any input. The rule is kept, and applies if you use an X11 session |
| Windows, macOS | implemented (Raw Input, IOHIDManager), compile-checked only, not tested on hardware |

Where a rule can't apply, `input-devices`, `doctor`, the lists and the add-time
note say so. Autoscroll and gestures cannot be set per device.

## Per-app rules: `--app`

Apps are looked up on the computer being controlled. Windows, macOS and X11
windows report their app; on GNOME and KDE **Wayland-native windows nothing
can say which app has the focus**, so rules with `--app` do not apply there
(never wrongly: unknown means the rule is off) and `status` says so.

## This computer's own keys (experimental, Windows and macOS only)

```
one-mk keymap local on            # also apply the key and button rules here
one-mk keymap add capslock esc --local
one-mk keymap local off
```

Experimental and **not yet tested on real hardware**: it changes how your own
keyboard behaves. It is off by default, only offered on Windows and macOS (it
is not available on Linux), and Ctrl+Alt+Shift+Esc turns it off. If it ever
misbehaves, run `one-mk keymap local off` from another login or computer, or
stop one-mk (it restores the original mappings). A Mac can map Caps Lock to
one other key only, and Fn, media and function keys are not available there.

## Pointer overlay and the GNOME pointer extension

* `one-mk pointer-overlay auto|always|never`: draw the pointer on a computer
  with no mouse while another computer controls it (drawn on X11, Windows and
  macOS; not available on Wayland).
* `one-mk pointer-extension install|enable|status|uninstall`: an optional
  GNOME Shell extension that hides this computer's pointer while the mouse is on
  another computer. It is tried on **GNOME Shell 50 only**; `install` refuses
  other versions and says so. Without it the pointer stays drawn at the edge.
  It is installed into your own extensions folder, never enabled for you, and
  GNOME loads it at the next login.

## `settings.toml`

The commands above write the `[input]` tables; they can also be edited by hand:

```toml
[input.keymap]
tap_ms = 200

[[input.keymap.rule]]
from = "capslock"
tap = "esc"
hold = "lctrl"

[[input.keymap.rule]]
from = "lalt"
to = "lmeta"
scope = { os = "mac" }                 # or { peer = "laptop" }, { device = "..." }

[[input.pointer]]
speed = 1.5
app = "*game*"

[[input.scroll]]
natural = "auto"
modifiers = { shift = "horizontal", alt = "x5" }

[[input.button]]
from = 4
to = "back"
```

A mistake in the file costs only what it is in. one-mk reads everything else,
and `keymap check`, `status` and `doctor` name the line and what was left out
(`settings.toml line 14: [[input.scroll]] entry 2 is ignored: ...`). Commands
that change settings refuse until the line is fixed, so a hand edit is never
overwritten. Out-of-range numbers are clamped and reported. Limits: 8 layers,
16 steps in a sequence, 4 modifiers plus a key per chord.

## What is tested

Key, button and scroll rules, tap/hold, layers and the "nothing stays held"
guarantee are tested with random event streams and with two real daemons over a
loopback link. Windows and macOS code paths (native capture, device correlation,
local rules, letters, cursor overlay) are compile-checked only; Linux
GNOME/KDE/wlroots capture and Bluetooth devices are not covered by the
automated tests. Treat anything labelled experimental accordingly.
