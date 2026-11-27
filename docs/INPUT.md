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
one-mk keymap add f15 os:overview                   # Mission Control, Task View or Activities
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
`forward`, an OS shortcut such as `os:overview`, or an action: `switch next`,
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
one-mk buttons map 4 os:next-desktop
one-mk buttons map 3 ctrl+w
one-mk buttons map 5 none --app "code*"
one-mk buttons unmap 4
one-mk buttons list
```

Buttons are numbered 1 left, 2 right, 3 middle, 4 back, 5 forward, 6 to 8
extra. A button can become another button, a key, a chord, a sequence, `back`
or `forward`, an OS shortcut, an action, or `none`. **Left and right cannot be made to do
nothing or hold a layer** (the other computer would have no clicks); they are
refused, and `keymap check` flags such a rule written by hand.

## System actions

Use `os:NAME` as a single output in a key rule, either tap/hold slot, a
mouse button rule or a swipe gesture. The action taps the receiving
computer's shortcut once per press, without auto-repeat. A single
`os:switch-app` press toggles between the two most recent apps. Ctrl/Cmd
swapping still sends the shortcut shown below. **Also on this computer**
uses the local OS for supported key and button rules on Windows and macOS.
An unavailable action sends nothing for that OS. The save command and
`one-mk keymap check` print a note with the supported scopes. A primary
mouse button requires a shortcut on every OS in its scope. Bare names
such as `copy`, `paste` and `undo` still mean the corresponding physical keys.

On macOS, keep the defaults enabled in **System Settings → Keyboard →
Keyboard Shortcuts → Mission Control** for Spaces, Mission Control, App
Exposé and Show Desktop. oneMK sends the standard F11 key for Show Desktop.
Apple keyboards may require Fn to produce F11 when pressed by hand. The
Apps panel or Launchpad can be opened manually with Apple’s Fn shortcut
or a custom binding. The system action is unavailable on macOS.
On Linux, the desktop mappings follow GNOME. To use Show Desktop, bind
Super+D to **Hide all normal windows** in GNOME Keyboard Shortcuts.
The desktop-switching actions use Ctrl+Alt+Left/Right, which preserves
the side-swipe preset's bindings for desktops such as Cinnamon and Xfce.
GNOME also binds these chords alongside Super+Page Up/Down in its
default schema. Customized shortcuts may differ. [GNOME 40 defaults][gnome-40-schema],
[current GNOME defaults][gnome-schema].

| Setting | macOS | Windows | Linux (GNOME) | References |
|---|---|---|---|---|
| `os:next-desktop` | Ctrl+→ | Ctrl+Win+→ | Ctrl+Alt+→ | [Apple Spaces][apple-spaces], [Microsoft][ms-keys], [GNOME defaults][gnome-schema] |
| `os:previous-desktop` | Ctrl+← | Ctrl+Win+← | Ctrl+Alt+← | [Apple Spaces][apple-spaces], [Microsoft][ms-keys], [GNOME defaults][gnome-schema] |
| `os:overview` | Ctrl+↑, Mission Control | Win+Tab, Task View | Super tap, Activities | [Apple Mission Control][apple-mission], [Microsoft][ms-keys], [GNOME][gnome-keys] |
| `os:app-windows` | Ctrl+↓, App Exposé | Unavailable | Super+key above Tab, switch windows of the same app | [Apple Mission Control][apple-mission], [Microsoft][ms-keys], [GNOME][gnome-keys] |
| `os:show-desktop` | F11 | Win+D | Super+D, needs a binding on stock GNOME | [Apple][apple-keys], [Microsoft][ms-keys], [GNOME settings][gnome-settings] |
| `os:app-launcher` | Unavailable | Win tap, Start | Super+A | [Apple][apple-keys], [Microsoft][ms-keys], [GNOME][gnome-keys] |
| `os:search` | Cmd+Space, Spotlight | Win+S | Super tap, then type | [Apple][apple-keys], [Microsoft][ms-keys], [GNOME overview][gnome-overview] |
| `os:screenshot-area` | Cmd+Shift+4 | Win+Shift+S | Print, then select an area | [Apple][apple-keys], [Microsoft][ms-keys], [GNOME screenshots][gnome-shot] |
| `os:screenshot-tool` | Cmd+Shift+5 | Win+Shift+S | Print opens the screenshot UI on GNOME 42+ | [Apple][apple-keys], [Microsoft][ms-keys], [GNOME screenshots][gnome-shot] |
| `os:lock-screen` | Ctrl+Cmd+Q | Win+L | Super+L | [Apple][apple-keys], [Microsoft][ms-keys], [GNOME][gnome-keys] |
| `os:force-quit` | Cmd+Option+Esc | Ctrl+Shift+Esc, Task Manager | Unavailable | [Apple][apple-keys], [Microsoft][ms-keys], [GNOME settings][gnome-settings] |
| `os:full-screen` | Ctrl+Cmd+F in supported apps | F11 in supported apps | F11 in supported apps | [Microsoft Edge][ms-edge], [GNOME Terminal][gnome-terminal] |
| `os:emoji-picker` | Ctrl+Cmd+Space | Win+. | Unavailable as a desktop shortcut | [Apple][apple-keys], [Microsoft][ms-keys], [GNOME settings][gnome-settings] |
| `os:switch-app` | Cmd+Tab | Alt+Tab | Alt+Tab | [Apple][apple-keys], [Microsoft][ms-keys], [GNOME defaults][gnome-schema] |
| `os:hide-app` | Cmd+H | Unavailable | Unavailable | [Apple][apple-keys], [Microsoft][ms-keys], [GNOME settings][gnome-settings] |
| `os:close-window` | Cmd+W | Alt+F4 | Alt+F4 | [Apple][apple-keys], [Microsoft][ms-keys], [GNOME settings][gnome-settings] |
| `os:quit-app` | Cmd+Q | Unavailable | Unavailable | [Apple][apple-keys], [Microsoft][ms-keys], [GNOME settings][gnome-settings] |
| `os:copy`, `os:cut`, `os:paste` | Cmd+C / X / V | Ctrl+C / X / V | Ctrl+C / X / V | [Apple][apple-keys], [Microsoft][ms-keys], [GTK][gtk-text] |
| `os:undo`, `os:redo` | Cmd+Z / Cmd+Shift+Z | Ctrl+Z / Ctrl+Y | Ctrl+Z / Ctrl+Shift+Z | [Apple][apple-keys], [Microsoft][ms-keys], [GTK][gtk-text] |
| `os:select-all`, `os:find`, `os:save` | Cmd+A / F / S | Ctrl+A / F / S | Ctrl+A / F / S | [Apple][apple-keys], [Microsoft Edge][ms-edge], [GTK][gtk-text], [Chrome][chrome-keys] |
| `os:new-tab`, `os:close-tab` | Cmd+T / W | Ctrl+T / W | Ctrl+T / W | [Apple Safari][apple-safari], [Microsoft][ms-keys], [Chrome][chrome-keys] |
| `os:next-tab`, `os:previous-tab` | Ctrl+Tab / Ctrl+Shift+Tab | Ctrl+Tab / Ctrl+Shift+Tab | Ctrl+Tab / Ctrl+Shift+Tab | [Apple Safari][apple-safari], [Microsoft][ms-keys], [Chrome][chrome-keys] |

Editing and tab shortcuts depend on the focused app. For example,
GNOME Terminal uses Ctrl+Shift+C and Ctrl+Shift+V for copy and paste.

[apple-keys]: https://support.apple.com/en-us/102650
[apple-spaces]: https://support.apple.com/guide/mac-help/-mh14112/mac
[apple-mission]: https://support.apple.com/guide/mac-help/view-open-windows-spaces-mission-control-mh35798/mac
[apple-safari]: https://support.apple.com/guide/safari/keyboard-shortcuts-and-gestures-cpsh003/mac
[ms-keys]: https://support.microsoft.com/en-us/accessibility/windows/keyboard-shortcuts-in-windows
[ms-edge]: https://support.microsoft.com/en-us/edge/keyboard-shortcuts-in-microsoft-edge
[gnome-settings]: https://help.gnome.org/gnome-help/keyboard-shortcuts-set.html
[gnome-keys]: https://help.gnome.org/gnome-help/shell-keyboard-shortcuts.html
[gnome-overview]: https://help.gnome.org/gnome-help/shell-overview.html
[gnome-shot]: https://help.gnome.org/gnome-help/screen-shot-record.html
[gnome-terminal]: https://help.gnome.org/gnome-terminal/adv-keyboard-shortcuts.html
[gnome-40-schema]: https://gitlab.gnome.org/GNOME/gsettings-desktop-schemas/-/blob/40.0/schemas/org.gnome.desktop.wm.keybindings.gschema.xml.in
[gnome-schema]: https://gitlab.gnome.org/GNOME/gsettings-desktop-schemas/-/blob/main/schemas/org.gnome.desktop.wm.keybindings.gschema.xml.in
[gtk-text]: https://gnome.pages.gitlab.gnome.org/gtk/gtk4/class.Text.html
[chrome-keys]: https://support.google.com/chrome/answer/157179

## Which key is that? `input-devices detect`

A keyboard key that isn't on the drawn keyboard (a macro key, `F13`, a key
with no name), or a mouse button the drawn mouse doesn't have, can be changed
once you know what it is to oneMK. Press it:

```
one-mk input-devices detect               # press the key or button within 30 s
one-mk input-devices detect --timeout 60 --keys-only
one-mk input-devices detect --json        # one line of JSON, for programs
```

It prints the key as rules name it (`f13`, `code:432` for a key with no
name, `button7`), with the modifiers held with it: a mouse button that types
a shortcut comes out as that shortcut (`ctrl+alt+tab`), and a rule for that
shortcut, made for that mouse with `--device`, changes only what the mouse
sends. Where this computer can tell devices apart it also names the keyboard
or mouse, and prints the `keymap add` or `buttons map` commands for every
device and for that one. In the window, **Detect key** (Keyboard) and
**Detect button** (Mouse) do the same and open the key or button's editor,
with "Only on <device>" offered; mouse buttons 6 to 8 are listed under
**Other buttons**.

A mouse button is detected on its own: modifiers held with it (Ctrl held
while pressing Back) are said, and left out, as a button's rule
(`buttons map`) is for the button alone. Where this computer can't use a
rule for what was pressed, it says so instead of offering one ("detected,
but ... can't be changed on this system"): mouse buttons 6 to 8 on macOS and
Windows (nothing there forwards or remaps them), and, with local key rules
on, keys local remapping can't apply yet (a rule for one would pause local
remapping). Where the device comes on another path (Windows, macOS), a press
is matched with the device reports around it by the system's own event
times; if two devices pressed that key at once, the device is left unknown
rather than guessed.

Detecting changes nothing: the key still does what it does (Esc alone
cancels instead). Key repeats, keys already down when it started (a
modifier held from before still counts with the next key, so Ctrl held
then Tab is `ctrl+tab`), and input that programs inject are ignored, both
while the pointer is here and while this computer controls another one. It stops when a key is pressed, when the
time is up, on Esc, when the command is stopped (Ctrl+C) or the window's
Cancel is used, and when one-mk stops; while nothing is being detected it
costs nothing.

| desktop | what can be detected |
|---|---|
| Linux, X11 session | every key and button the X server reports, with its device (tried against a private X server) |
| Linux, GNOME, KDE or wlroots on Wayland | only keys pressed **while oneMK controls another computer** (the desktop shows it no input otherwise): move the pointer onto another computer's screen, then press the key. The window also takes the keys it sees itself (not the keypad, Fn, media keys or a mouse's extra keys) |
| Windows | keys and buttons 1 to 5, device from Raw Input; compile-checked only |
| macOS | keys (Caps Lock and media keys too) and buttons 1 to 5, device from IOHIDManager; buttons 6 to 8 are named but can't be changed; compile-checked only |

Some presses never reach oneMK and can't be detected: keys a keyboard handles
itself (usually Fn), buttons a mouse only reports to its maker's software, and
mouse buttons past the eighth (named, but they can't be changed). On X11,
buttons past the X server's ninth are not forwarded, so they can't be changed
there either.

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
one-mk gestures add button4 left os:previous-desktop --threshold 60
one-mk gestures add button5 up "ctrl+super+left" --os windows   # not middle: autoscroll uses it while on
one-mk gestures list
one-mk gestures remove button4 left
```

* Autoscroll: click the toggle button (middle by default) and move away; the
  further you move, the faster it scrolls. Click again, press Esc or any other
  button, or leave the computer to stop. The toggle click is not sent.
* Gesture: hold the button and move past the threshold (1 to 10000 motion
  units, 60 by default) in a direction; the output happens once. A short
  click still clicks. Esc cancels a swipe. Use `os:previous-desktop` and
  `os:next-desktop` for desktop switching with Ctrl/Cmd swap compensation.
  Older `desktop-left` and `desktop-right` settings retain their original chords.
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
one other key only. Fn, media and function keys are unavailable as local
triggers. `os:show-desktop` can output the standard F11 key.

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

## Keys already held when the pointer crosses

Hold Ctrl (or any key), push the pointer across the edge, and let go while the other computer
has the pointer: what happens to that key depends on the desktop.

- **Windows and macOS:** the key's release is let through to this computer's own programs, so none
  is left thinking it is still down. Modifiers held at the crossing are told to the other computer
  (that is what lets a modifier be the key that crosses) but are not pressed there.
- **X11:** a program that saw the key go down never sees it go up while the other computer has
  the pointer, because X's keyboard grab sends that release to oneMK, which ignores it (it was not
  pressed for the other computer, so it is not released there either). Programs normally put
  their idea of the held keys right again when the grab starts and ends (X tells them the focus
  was taken and given back), and every modifier state comes from the server, so this only matters
  for a program that keeps its own table of held keys and ignores those notices (a game, a remote
  desktop viewer): it can think the key is still down until you press and let go of it once.
  oneMK does not send that release itself and does not wait for the keys to come up before
  crossing. A fake release could undo a press another program has since made, and waiting would
  break crossing with a held modifier.

## What is tested

Key, button and scroll rules, tap/hold, layers and the "nothing stays held"
guarantee are tested with random event streams and with two real daemons over a
loopback link. Windows and macOS code paths (native capture, device correlation,
local rules, letters, cursor overlay) are compile-checked only; Linux
GNOME/KDE/wlroots capture and Bluetooth devices are not covered by the
automated tests. Treat anything labelled experimental accordingly.
