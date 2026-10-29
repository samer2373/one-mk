# A pointer on a computer with no mouse (plan 3A)

When another computer controls this one and this one has **no mouse
attached** (a laptop used with the lid closed, a headless box, a tablet
PC), the OS often draws no pointer at all. The person controlling it then
can't see where the pointer is. Lan Mouse and Mouse Without Borders both
have this problem; Mouse Without Borders works around it with a "Draw mouse
cursor" option. one-mk draws a small arrow of its own where needed.

This page says what each OS does, how sure we are, and what one-mk does
about it. **Verified** means seen running on this dev box (Ubuntu 26.04,
GNOME 50 on Wayland, plus a private Xvfb). **Documented** means the vendor's
own documentation says so. **Reported** means user reports only. Nothing
here was run on Windows or macOS: that code is compiled, not tested.

## What each OS does

### Windows

- **Documented.** `ShowCursor`: "The cursor is displayed only if the display
  count is greater than or equal to 0. If a mouse is installed, the initial
  display count is 0. If no mouse is installed, the display count is -1."
  ([ShowCursor](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showcursor))
  So with no mouse the cursor starts hidden, and injected motion
  (`SendInput`) moves a cursor nobody sees.
- **Documented.** `GetCursorInfo` reports `CURSOR_SHOWING` (1) or, from
  Windows 8, `CURSOR_SUPPRESSED` (2): "the system is not drawing the cursor
  because the user is providing input through touch or pen instead of the
  mouse"; 0 means hidden.
  ([CURSORINFO](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-cursorinfo))
- **Documented.** Mouse Without Borders' known issues: "The mouse pointer
  might be invisible if there is no physical mouse attached to the machine.
  Plug in an unused mouse or turn on Mouse Keys in Control Panel." Its
  setting "Draw mouse cursor: Attempt to draw the mouse cursor on machines
  that have no physical peripheral attached."
  ([Mouse Without Borders](https://learn.microsoft.com/windows/powertoys/mouse-without-borders))
- `GetSystemMetrics(SM_MOUSEPRESENT)` is "nonzero if a mouse is installed",
  the same notion of "installed" as the display count above.
- **Not verified:** whether a laptop with its lid closed still counts its
  touchpad as an installed mouse (probably: the device stays enumerated),
  and whether virtual HID devices (some drivers, VMs) count. Where they
  do, Windows shows the cursor and `auto` correctly draws nothing; if the
  cursor is still missing, `one-mk pointer-overlay always` draws it anyway.

### macOS

- **Reported.** Users of Synergy/Deskflow report an invisible pointer on a
  Mac client with no mouse of its own, fixed by plugging in a mouse
  ([Symless forum](https://forums.symless.com/topic/6914-cursor-not-visible-on-client-machine-without-its-own-mouse/)).
  We found no Apple documentation of this behaviour.
- `CGDisplayShowCursor` and `CGAssociateMouseAndMouseCursorPosition(true)`
  are the public calls that ask for the pointer to show and to follow
  moves; whether they help when no mouse is attached is **not verified**.
- `CGCursorIsVisible` is deprecated ("no longer supported"), so there is no
  reliable way to ask whether the pointer shows. one-mk therefore decides
  on macOS from whether a pointing device is registered with IOKit.
- AppKit windows can only be created on a process's main thread, and the
  daemon's main thread runs its async runtime. So the arrow is drawn by a
  tiny helper process (see below), not by the daemon.

### Linux on X11

- The X server always draws its pointer: the virtual core pointer exists
  with or without a physical mouse, and XTest moves it. Applications can
  hide it (XFixes `HideCursor`, a blank cursor), as on any OS, but nothing
  hides it for lack of a mouse. **Verified** on Xvfb (which has no mouse at
  all): XTest motion moves the server's pointer; the existing X11 backend
  tests rely on that.
- So `auto` never draws on X11. `always` does, cheaply (see below), and that
  is **verified** on Xvfb.

### Linux on Wayland (GNOME, KDE, wlroots)

- The compositor draws the pointer. A client can't put a surface at an
  arbitrary screen position (GNOME, KDE) and can't draw over everything, so
  one-mk can't draw a pointer there: `one-mk doctor` says "not available
  on this desktop".
- According to its commit history, GNOME (Mutter) hides the pointer when a
  touchscreen (and, on Wayland, a tablet) is being used and shows it for
  pointing devices
  ([Mutter commit](https://mail.gnome.org/archives/commits-list/2022-January/msg04725.html));
  KWin's DRM backend shows it only when a pointer device is available
  ([KWin change](https://phabricator.kde.org/R108:b5b2adc5a26e51445342a95d93286b5b81b3841f)).
  The libei / RemoteDesktop virtual pointer that one-mk injects through is a
  pointer device to the compositor, so it should count. **Not verified:**
  one-mk never injects into the live desktop of the dev box, and there is
  no headless GNOME/KDE test setup here.
- wlroots compositors (Sway, Hyprland) draw the pointer of any seat with the
  pointer capability; the virtual pointer (`zwlr_virtual_pointer_v1`) gives
  the seat that capability. **Not verified** here.

## What one-mk does

`[pointer] draw_when_no_mouse` in `settings.toml`, set with
`one-mk pointer-overlay auto|always|never` (no value: show the setting and
what it means on this computer). `auto` is the default.

The decision (`crates/proto/src/overlay.rs`, pure and unit-tested) draws
only while a peer controls this computer with positions (not gaming mode's
raw motion), where drawing is possible, and:

| | OS pointer shown | suppressed (Windows, touch/pen) | hidden or unknown, no pointing device | hidden or unknown, a pointing device |
|---|---|---|---|---|
| `auto` | no | yes | yes | no (an app hid it on purpose) |
| `always` | yes | yes | yes | yes |
| `never` | no | no | no | no |

Hysteresis: control starting or ending, gaming mode, the setting and a
mouse being plugged in or out apply at once; a change only the pointer
probe sees must hold for 750 ms, so a flickering probe never makes the
arrow blink.

The arrow (`crates/input/src/cursoroverlay*.rs`): a plain 12 x 19 pointer
shape, white with a black outline, tip at the hot spot, drawn at 1x, 2x or
3x for the display scale.

- **Windows:** a layered window (`WS_EX_LAYERED | WS_EX_TRANSPARENT |
  WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`) on a thread of its
  own with its own message loop, moved with `SetWindowPos` (asynchronous)
  on each injected move. It registers for mouse device arrivals and hides
  itself the moment one arrives; the OS pointer is used from then on.
  The probe is `SM_MOUSEPRESENT` plus `GetCursorInfo`. **Compiled only.**
- **macOS:** the same program started as a helper, running `NSApplication`
  as an accessory (no Dock icon) with one borderless window at the
  screen-saver level that ignores the mouse and joins every Space, showing
  the system arrow image. It reads positions from a pipe and exits when the
  pipe closes. `CGDisplayShowCursor` is tried first. The probe asks IOKit
  for HID mice, pointers and touch pads. **Compiled only.**
- **X11:** an override-redirect window shaped like the arrow with an empty
  input shape (clicks go through), painted from a background pixmap by the
  server: no thread, no event loop. **Verified on Xvfb** (shape, position,
  following moves, removal, repeated cycles).
- **Wayland:** nothing (see above).

Zero idle cost: nothing exists until a peer takes control and the decision
says draw: no window, no thread, no helper process, no timer. Everything is
driven by the daemon's injected events; the OS is asked at most every
500 ms, and only while a peer moves the pointer. When control ends, the
setting changes to `never`, or a mouse appears, the window (and its thread
or helper) goes. The lifecycle is tested with a fake renderer: created on
control, destroyed after, nothing left after 200 cycles, no retry storm
when creating fails.

`one-mk status` says "one-mk draws the pointer here" while the arrow is up;
`one-mk doctor` has a "Pointer without a mouse" line: available (Windows,
macOS), not needed (X11), or not available on this desktop (Wayland).
