# Broadcast: type on every computer at once

While broadcast is on, everything you type, click and scroll on this
computer goes to every connected paired computer at the same time. Use it to
set up several computers the same way, or to run the same command on all of
them. (Mouse Without Borders calls this "multi-machine" mode.)

```
one-mk broadcast on        # start
one-mk broadcast off       # stop
one-mk broadcast toggle    # what the keyboard shortcut does
```

Default keyboard shortcut: **Ctrl+Alt+B** (**Ctrl+Alt+Shift+B** on Windows,
see below). Change it with `one-mk hotkeys --broadcast <chord>` or turn it off
with `one-mk hotkeys --broadcast none`. The tray and the window's Desk tab
have a "Broadcast to all computers" checkmark/switch (shown while a computer
is connected); while it is on, the Desk headline says so and the window's
main button becomes "Stop broadcast".

## What happens

| | |
|---|---|
| Who gets the input | Every paired computer that is connected and has said what its screens are **when broadcast starts**. `one-mk status` says "Broadcasting to 3 computers: A, B and C". |
| This computer | Gets none of it: the mouse and keyboard are captured, as during a normal visit to another computer. When broadcast ends the pointer comes back to the middle of this computer's main screen. |
| Keys, buttons, scrolling | Sent to every computer. Each computer gets its own copy of the key mapper (`[input]`) and its own Ctrl/Cmd swap, resolved for that computer as in a normal visit, so a Mac and a PC can both receive the same shortcut correctly. |
| Pointer movement | Each computer has its own pointer, starting at the centre of its main screen. Every movement of your mouse moves each of them by the same amount (after that computer's pointer speed setting), and each stops at the edges of its own screens. The pointer never leaves a computer during broadcast: there is no edge back home. |
| The other computers | See a normal controller. They cannot tell it is a broadcast, and cannot start one. |

## Starting

Broadcast starts only from this computer: `one-mk broadcast on|toggle`, the
keyboard shortcut, the tray or the window. Nothing a connected computer sends can start
it.

- With no other computer connected it does not start, and says so: "No other
  computer is connected, so there is nothing to broadcast to."
- Like `one-mk switch`, it asks the input backend to start capturing. If the
  capture has not started after 1 second, broadcast does not start and the
  command says why.
- If the pointer is on another computer (a normal visit) or held at an edge,
  that visit ends (everything held there is released) and broadcast starts at
  once.
- If another computer controls this one, it lets go first (as with
  `one-mk switch`).
- A desktop notification says it started, and to which computers.

## Who stays in

- **A computer that connects during broadcast is not added.** Broadcast goes
  to the computers that were there when it started, so it is predictable. Stop
  and start it again to include it.
- **A computer that disconnects drops out.** Everything this computer was
  holding down for it is forgotten here, and that computer releases it by
  itself (its 750 ms lease ends). If it was the last one, broadcast ends.
- **A computer whose user takes control back** (its own mouse or keyboard on
  Windows and macOS, `one-mk takeback` anywhere) drops out the same way.
- **A computer that let go after a minute without input** (its idle rule) is
  not dropped: it is entered again with your next key, click, scroll or
  movement, and that input waits until it accepts. This is how broadcast can
  sit idle for longer than a minute while you read the output of a command.
  (A computer that let go less than 58 s after the last input sent to it is
  taken to have been taken back by its user.)
- A computer that is busy (another computer controls it) or does not accept
  within 750 ms drops out.

## Stopping

Broadcast ends with any of:

- the keyboard shortcut again, `one-mk broadcast off|toggle`, the tray item, or
  the Stop button in the window (see "Known limitations");
- `one-mk switch` to anywhere (`here`, `next`, a computer) and the jump, next
  and home shortcuts: broadcast ends first, then the switch happens;
- **Ctrl+Alt+Shift+Esc**, which always brings the pointer home;
- this computer's screen locking (and `one-mk lock-all`);
- the input capture ending by itself (the system took it away);
- **15 minutes without any input**, so it can't be forgotten on
  (`[broadcast] idle_off_minutes` in `settings.toml`, 0 = never);
- the last computer dropping out;
- one-mk stopping.

A desktop notification says it stopped, and why.

## No stuck keys

Every computer in the broadcast is its own visit, with its own record of what
is held, its own key mapper state and its own epoch. When broadcast ends, or
one computer drops out, the end of each visit runs exactly as for a normal
visit: the key mapper lets go of what it pressed, and the computer is told the
visit is over, upon which it releases everything it holds (keys and buttons)
by its own record. A computer that is gone releases by itself when its lease
runs out. A computer that is entered again after its idle rule starts with
nothing held, so a key released later is not sent to it.

Keyboard shortcuts are matched on the physical keys before anything is sent,
as in a normal visit: the shortcut's last key never reaches any computer, and
the modifiers that did reach them are released when broadcast ends.

The tests check this: the same key-state invariant as a single visit,
multiplied by the number of computers, with random joins (idle re-entry) and
drops (`app/broadcast.rs`), and three real daemons on loopback (one
controller, two computers).

## What broadcast does not do

- **The clipboard does not follow.** In a normal visit the clipboard goes to
  the computer the pointer arrives on; with several at once there is no single
  one, so broadcast never pushes the clipboard. Use `one-mk clip-send <name>`.
- **No gaming mode, no rules for apps.** Pointer movement is always sent as
  positions (it is clamped per computer), and `[input]` rules that name an app
  don't apply (the computers are not asked which app has their focus). Rules
  for a computer, its OS or everyone apply as usual.
- **No hooks on this computer.** `[hooks]` `leave`/`enter` on this computer
  don't run for broadcast (it is not "the pointer is on computer X"); the other
  computers run their own `enter`/`leave` hooks as for any controller.
- **Key mapper actions run once.** A key mapped to `switch`, `lock-pointer`,
  `lock-all` or a command runs that action once, not once per computer (a
  mapped scroll is sent to each computer). A `switch` ends broadcast.

## Settings

```toml
[broadcast]
# Stop broadcasting after this many minutes without any key, click,
# scroll or movement. 0 = never.
idle_off_minutes = 15

[hotkeys]
broadcast = "ctrl+alt+b"     # "none" turns the shortcut off
```

A `settings.toml` from before broadcast existed gets both defaults (checked
with files written by the previous build; that build also still reads the
new file). If such a file already gives Ctrl+Alt+B to another shortcut, that
shortcut keeps it: broadcast then has no shortcut, `one-mk hotkeys` says so,
and every other shortcut keeps working. (A broadcast chord you choose that
clashes with another is refused, as for any shortcut.)

**Why Ctrl+Alt+Shift+B on Windows:** Windows reports AltGr as Ctrl+Alt, and
AltGr+B types `{` on Hungarian keyboards, so a Ctrl+Alt+B shortcut would take
that character away while the pointer is on this computer. (The same reason
the jump shortcuts add Shift there.) On Linux, the first start after upgrading
asks the desktop to approve the new shortcut (GNOME shows its shortcut
dialog).

## Cost

Nothing runs for broadcast while it is off: no timer, no task. While it is on,
each computer gets the same traffic as a normal visit (a keepalive every 20 ms
while idle, resends after movement), so N computers cost N times one visit.
Motion is applied to each computer's pointer from one reusable buffer: no
allocation per event beyond what a single visit does. The auto-off is one
deadline in the daemon's existing timer, not a separate timer.

Measured (debug build, Linux, X11 backend in Xvfb, two computers, idle, 10 s
samples): broadcast off, the controller used 10 to 30 ms of CPU per 10 s, the
same as the previous build (30 ms per 20 s both); broadcast on, 260 ms per
10 s on the controller and 170 ms on each controlled computer (the 20 ms
keepalives); off again, back to 10 ms. Memory: 34 MB resident, against
32.4 MB for the previous build after the same start (debug builds).

## Status and visibility

- `one-mk status` starts with "Broadcasting to 3 computers: A, B and C.
  Everything you type, click and scroll goes to all of them."
- The tray headline says "Broadcasting to 3 computers" and the item
  "Broadcast to all computers" is checked.
- The window: the Desk headline "Broadcasting to 3 computers" and to whom,
  the main button "Stop broadcast", and the Desk switch on.
- Notifications at start and end (checked on GNOME; macOS untested; on
  Windows one-mk has no notifications yet, so only the status, tray and
  window show it).
- `one-mk doctor` has a Broadcast check: a warning while it is on (so typing
  that goes everywhere is never a mystery), otherwise its shortcut and when
  it stops by itself, and "untested" where it has not been tried.

## Known limitations

- While broadcast is on, this computer's mouse and keyboard are captured, so
  the window's Stop button and the tray can't be clicked with them. Stop with
  the shortcut, Ctrl+Alt+Shift+Esc, or `one-mk broadcast off` from another
  terminal or over SSH. The button is there for input that isn't captured
  (a touch screen, accessibility tools) and as a visible state.
- Broadcast was tested on Linux with the null input backend (three daemons on
  loopback, in the tests and with the real binaries) and with real X11 capture
  in a private Xvfb (keys, Shift+click, scrolling, movement, the shortcut and
  Ctrl+Alt+Shift+Esc). **Windows, macOS, wlroots and the GNOME/KDE portal are
  untested** with broadcast (the Windows and macOS parts of the window, tray
  and input are compile-checked; the daemon's notification code for them is
  not, as the daemon can't be cross-compiled here). `one-mk status` (on
  Windows and macOS) and `one-mk doctor` say so.
- A computer that is killed or loses the network drops out only when the
  connection notices (a few seconds); until then it is still listed.
- The other computers show no sign that it is a broadcast (by design).
- A computer whose user takes control back 58 s or more after the last input
  sent to it can't be told from its idle rule letting go, so it is entered
  again with your next input (its user can take it back again, or stop the
  broadcast). Telling the two apart needs a reason in the protocol's
  `Revoked` message, which older versions would not send.
- At most 9 computers make sense for the shortcuts; broadcast itself has no
  limit, but each computer adds a visit's traffic.
